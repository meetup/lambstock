//! AWS Lambda stock management

extern crate futures;
extern crate rusoto_lambda;
extern crate rusoto_resourcegroupstaggingapi;
#[macro_use]
extern crate lazy_static;
extern crate tokio;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate structopt;
extern crate humansize;

// Std lib
use std::collections::{BTreeSet, HashMap};
use std::error::Error as StdError;
use std::str::FromStr;

// Third party
use futures::future::{self, Future};
use futures::sync::oneshot::spawn;
use humansize::{file_size_opts as options, FileSize};
use rusoto_lambda::{FunctionConfiguration, Lambda, LambdaClient, ListFunctionsError,
                    ListFunctionsRequest};
use rusoto_resourcegroupstaggingapi::{GetResourcesError, GetResourcesInput,
                                      ResourceGroupsTaggingApi, ResourceGroupsTaggingApiClient,
                                      ResourceTagMapping, Tag, TagFilter};
use structopt::StructOpt;
use tokio::runtime::Runtime;

mod error;
use error::Error;

lazy_static! {
    static ref FALLBACK_RUNTIME: Runtime = Runtime::new().expect("failed to create runtime");
}

fn parse_key_val<T, U>(s: &str) -> Result<(T, U), Box<StdError>>
where
    T: FromStr,
    T::Err: StdError + 'static,
    U: FromStr,
    U::Err: StdError + 'static,
{
    let pos = s.find('=').ok_or_else(|| {
        format!("invalid KEY=value: no `=` found in `{}`", s)
    })?;
    Ok((s[..pos].parse()?, s[pos + 1..].parse()?))
}

/// CLI options
#[derive(StructOpt, PartialEq, Debug)]
#[structopt(name = "lambstock", about = "stock management for your AWS lambda")]
enum Options {
    #[structopt(name = "list", alias = "ls", about = "Lists lambdas")]
    List {
        #[structopt(short = "t", long = "tag", parse(try_from_str = "parse_key_val"))]
        tags: Vec<(String, String)>,
    },
    #[structopt(name = "tags", about = "Lists lambdas tags")]
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
        self.config
            .code_size
            .unwrap_or_default()
            .file_size(options::CONVENTIONAL)
            .unwrap_or_default()
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
) -> Box<Future<Item = Vec<FunctionConfiguration>, Error = ListFunctionsError> + Send> {
    Box::new(
        client
            .list_functions(ListFunctionsRequest {
                max_items: Some(100),
                marker,
                ..Default::default()
            })
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
) -> Box<Future<Item = Vec<ResourceTagMapping>, Error = GetResourcesError> + Send> {
    Box::new(
        client
            .get_resources(GetResourcesInput {
                resource_type_filters: Some(vec!["lambda:function".into()]),
                resources_per_page: Some(50),
                pagination_token,
                tag_filters: tag_filters.clone(),
                ..Default::default()
            })
            .and_then(move |result| {
                if let Some(token) = result.pagination_token.clone().filter(|s| !s.is_empty()) {
                    return future::Either::A(
                        tag_mappings(client, Some(token), tag_filters).map(|next| {
                            result
                                .resource_tag_mapping_list
                                .unwrap_or_default()
                                .into_iter()
                                .chain(next)
                                .collect()
                        }),
                    );
                }
                future::Either::B(future::ok(
                    result.resource_tag_mapping_list.unwrap_or_default(),
                ))
            }),
    )
}

fn render(funcs: Vec<Func>) {
    for func in funcs {
        println!(
            "{} {} {}",
            func.config.function_name.as_ref().unwrap(),
            func.config.runtime.as_ref().unwrap(),
            func.human_size()
        )
    }
}

fn main() -> Result<(), Error> {
    match Options::from_args() {
        Options::Tags => {
            let tags = tag_mappings(
                ResourceGroupsTaggingApiClient::new(Default::default()),
                Default::default(),
                None,
            ).map_err(Error::from);
            let names = tags.map(|mappings| {
                mappings.iter().fold(BTreeSet::new(), |mut names, mapping| {
                    for tag in mapping.tags.clone().unwrap_or_default() {
                        names.insert(tag.key.clone());
                    }
                    names
                })
            });
            Ok(println!(
                "{:#?}",
                spawn(names, &FALLBACK_RUNTIME.executor()).wait()?
            ))
        }
        Options::List { tags } => {
            let tag_mappings = tag_mappings(
                ResourceGroupsTaggingApiClient::new(Default::default()),
                Default::default(),
                Some(filters(tags)),
            ).map_err(Error::from);
            let lambdas = lambdas(LambdaClient::new(Default::default()), Default::default())
                .map_err(Error::from);
            let filtered = tag_mappings.join(lambdas).map(|(tags, lambdas)| {
                let lookup: HashMap<String, FunctionConfiguration> = lambdas
                    .into_iter()
                    .map(|config| {
                        (config.function_arn.clone().unwrap_or_default(), config)
                    })
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
            Ok(spawn(filtered.map(render), &FALLBACK_RUNTIME.executor())
                .wait()?)
        }
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
            }.human_size()
        )
    }
    #[test]
    fn cli_tags_to_filters() {
        let filters = filters(vec![("foo".into(), "bar".into())]);
        assert_eq!(
            filters,
            vec![
                TagFilter {
                    key: Some("foo".into()),
                    values: Some(vec!["bar".into()]),
                },
            ]
        )
    }
}
