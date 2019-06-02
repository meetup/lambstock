//! AWS Lambda stock management

use failure::Fail;
use futures::future::{self, Future};
use futures_backoff::Strategy;
use humansize::{file_size_opts as options, FileSize};
use rusoto_core::{credential::ChainProvider, request::HttpClient, RusotoError};
use rusoto_lambda::{
    FunctionConfiguration, Lambda, LambdaClient, ListFunctionsError, ListFunctionsRequest,
};
use rusoto_resourcegroupstaggingapi::{
    GetResourcesError, GetResourcesInput, ResourceGroupsTaggingApi, ResourceGroupsTaggingApiClient,
    ResourceTagMapping, Tag, TagFilter,
};
use std::{
    collections::{BTreeSet, HashMap},
    error::Error as StdError,
    fmt,
    io::{self, Write},
    process::exit,
    str::FromStr,
    time::Duration,
};
use structopt::StructOpt;
use tabwriter::TabWriter;
use tokio::runtime::Runtime;

mod error;
use crate::error::Error;

fn parse_key_val<T, U>(s: &str) -> Result<(T, U), Box<StdError>>
where
    T: FromStr,
    T::Err: StdError + 'static,
    U: FromStr,
    U::Err: StdError + 'static,
{
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=value: no `=` found in `{}`", s))?;
    Ok((s[..pos].parse()?, s[pos + 1..].parse()?))
}

#[derive(Debug, PartialEq)]
enum Sort {
    Name,
    Runtime,
    CodeSize,
}

impl Sort {
    fn variants() -> &'static [&'static str] {
        &["name", "runtime", "codesize"]
    }
}

impl FromStr for Sort {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "name" => Ok(Sort::Name),
            "runtime" => Ok(Sort::Runtime),
            "codesize" => Ok(Sort::CodeSize),
            _ => Err("no match"),
        }
    }
}

impl fmt::Display for Sort {
    fn fmt(
        &self,
        f: &mut fmt::Formatter,
    ) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Sort::Name => "name",
                Sort::Runtime => "runtime",
                Sort::CodeSize => "codesize",
            }
        )
    }
}

/// CLI options
#[derive(StructOpt, PartialEq, Debug)]
#[structopt(name = "lambstock", about = "stock management for your AWS lambda")]
enum Options {
    #[structopt(name = "list", alias = "ls", about = "List lambdas")]
    List {
        #[structopt(short = "t", long = "tag", parse(try_from_str = "parse_key_val"))]
        tags: Vec<(String, String)>,
        #[structopt(
            short = "s",
            long = "sort",
            default_value = "name",
            raw(possible_values = "&Sort::variants()", case_insensitive = "true")
        )]
        sort: Sort,
    },
    #[structopt(name = "tags", about = "List lambdas tags")]
    Tags,
}

/// A single lambda function with associated tags
#[derive(Debug, Default)]
struct Func {
    config: FunctionConfiguration,
    tags: Vec<Tag>,
}

impl Func {
    /// Return size of function for human display
    fn human_size(&self) -> String {
        self.code_size()
            .unwrap_or_default()
            .file_size(options::CONVENTIONAL)
            .unwrap_or_default()
    }

    fn name(&self) -> Option<String> {
        self.config.function_name.clone()
    }

    fn runtime(&self) -> Option<String> {
        self.config.runtime.clone()
    }

    fn code_size(&self) -> Option<i64> {
        self.config.code_size
    }
}

fn filters(tags: Vec<(String, String)>) -> Vec<TagFilter> {
    tags.into_iter().fold(Vec::new(), |mut filters, (k, v)| {
        filters.push(TagFilter {
            key: Some(k),
            values: Some(vec![v]),
        });
        filters
    })
}

fn lambdas(
    client: LambdaClient,
    marker: Option<String>,
) -> Box<Future<Item = Vec<FunctionConfiguration>, Error = RusotoError<ListFunctionsError>> + Send>
{
    let client_inner = client.clone();
    Box::new(
        backoff()
            .retry_if(
                move || {
                    client_inner.list_functions(ListFunctionsRequest {
                        max_items: Some(100),
                        marker: marker.clone(),
                        ..ListFunctionsRequest::default()
                    })
                },
                |err: &RusotoError<ListFunctionsError>| {
                    log::debug!("lambda api error {}", err);
                    match err {
                        RusotoError::Service(ListFunctionsError::TooManyRequests(_)) => true,
                        _ => false,
                    }
                },
            )
            .and_then(move |result| {
                if let Some(marker) = result.next_marker.clone().filter(|s| !s.is_empty()) {
                    return future::Either::A(lambdas(client, Some(marker)).map(|next| {
                        result
                            .functions
                            .unwrap_or_default()
                            .into_iter()
                            .chain(next)
                            .collect()
                    }));
                }
                future::Either::B(future::ok(result.functions.unwrap_or_default()))
            }),
    )
}

fn tag_mappings(
    client: ResourceGroupsTaggingApiClient,
    pagination_token: Option<String>,
    tag_filters: Option<Vec<TagFilter>>,
) -> Box<Future<Item = Vec<ResourceTagMapping>, Error = RusotoError<GetResourcesError>> + Send> {
    let client_inner = client.clone();
    let tag_filters_inner = tag_filters.clone();
    Box::new(
        backoff()
            .retry_if(
                move || {
                    client_inner.get_resources(GetResourcesInput {
                        resource_type_filters: Some(vec!["lambda:function".into()]),
                        resources_per_page: Some(50),
                        pagination_token: pagination_token.clone(),
                        tag_filters: tag_filters_inner.clone(),
                        ..GetResourcesInput::default()
                    })
                },
                |err: &RusotoError<GetResourcesError>| {
                    log::debug!("tagging api error {}", err);
                    match err {
                        RusotoError::Service(GetResourcesError::InvalidParameter(_)) => true,
                        _ => false,
                    }
                },
            )
            .and_then(move |result| {
                if let Some(token) = result.pagination_token.clone().filter(|s| !s.is_empty()) {
                    return future::Either::A(tag_mappings(client, Some(token), tag_filters).map(
                        |next| {
                            result
                                .resource_tag_mapping_list
                                .unwrap_or_default()
                                .into_iter()
                                .chain(next)
                                .collect()
                        },
                    ));
                }
                future::Either::B(future::ok(
                    result.resource_tag_mapping_list.unwrap_or_default(),
                ))
            }),
    )
}

fn render_funcs(
    funcs: &mut Vec<Func>,
    sort: Sort,
) {
    funcs.sort_unstable_by(|a, b| match sort {
        Sort::Name => a
            .name()
            .unwrap_or_default()
            .cmp(&b.name().unwrap_or_default()),
        Sort::CodeSize => a
            .code_size()
            .unwrap_or_default()
            .cmp(&b.code_size().unwrap_or_default()),
        Sort::Runtime => a
            .runtime()
            .unwrap_or_default()
            .cmp(&b.runtime().unwrap_or_default()),
    });
    let mut writer = TabWriter::new(io::stdout());
    for func in funcs {
        drop(writeln!(
            &mut writer,
            "{}\t{}\t{}",
            func.config.function_name.as_ref().unwrap(),
            func.config.runtime.as_ref().unwrap(),
            func.human_size()
        ));
    }
    drop(writer.flush())
}

fn render_tags(tags: BTreeSet<String>) {
    for tag in tags {
        println!("{}", tag)
    }
}

fn credentials() -> ChainProvider {
    let mut chain = ChainProvider::new();
    chain.set_timeout(Duration::from_millis(200));
    chain
}

fn lambda_client() -> LambdaClient {
    LambdaClient::new_with(
        HttpClient::new().expect("failed to create request dispatcher"),
        credentials(),
        Default::default(),
    )
}

fn backoff() -> Strategy {
    Strategy::exponential(Duration::from_millis(100))
        .with_max_retries(15)
        .with_jitter(true)
}

fn tags_client() -> ResourceGroupsTaggingApiClient {
    ResourceGroupsTaggingApiClient::new_with(
        HttpClient::new().expect("failed to create request dispatcher"),
        credentials(),
        Default::default(),
    )
}

fn main() {
    env_logger::init();
    let mut rt = Runtime::new().expect("failed to initialize runtime");
    let result = match Options::from_args() {
        Options::Tags => {
            let tags = tag_mappings(tags_client(), Default::default(), None)
                .map_err(Error::from);
            let names = tags.map(|mappings| {
                mappings.iter().fold(BTreeSet::new(), |mut names, mapping| {
                    for tag in mapping.tags.clone().unwrap_or_default() {
                        names.insert(tag.key.clone());
                    }
                    names
                })
            });
            rt.block_on(names.map(render_tags))
        }
        Options::List { tags, sort } => {
            let tag_mappings = tag_mappings(tags_client(), Default::default(), Some(filters(tags)))
                .map_err(Error::from);

            let lambdas = lambdas(lambda_client(), Default::default()).map_err(Error::from);
            let filtered = tag_mappings.join(lambdas).map(|(tags, lambdas)| {
                let lookup: HashMap<String, FunctionConfiguration> = lambdas
                    .into_iter()
                    .map(|config| (config.function_arn.clone().unwrap_or_default(), config))
                    .collect();
                tags.into_iter().fold(Vec::new(), |mut result, mapping| {
                    if let Some(config) = lookup.get(&mapping.resource_arn.unwrap_or_default()) {
                        result.push(Func {
                            tags: mapping.tags.unwrap_or_default(),
                            config: config.clone(),
                        });
                    }
                    result
                })
            });
            rt.block_on(filtered.map(|mut funcs| render_funcs(&mut funcs, sort)))
        }
    };
    if let Err(err) = result {
        for cause in Fail::iter_causes(&err) {
            eprintln!("{}", cause);
        }
        exit(1)
    }
}

#[cfg(test)]
mod tests {
    use super::{filters, Func, FunctionConfiguration, TagFilter};
    #[test]
    fn func_human_size() {
        assert_eq!(
            "1 KB",
            Func {
                config: FunctionConfiguration {
                    code_size: Some(1024),
                    ..Default::default()
                },
                ..Default::default()
            }
            .human_size()
        )
    }
    #[test]
    fn cli_tags_to_filters() {
        let filters = filters(vec![("foo".into(), "bar".into())]);
        assert_eq!(
            filters,
            vec![TagFilter {
                key: Some("foo".into()),
                values: Some(vec!["bar".into()]),
            }]
        )
    }
}
