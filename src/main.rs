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

// Third party
use futures::future::{self, Future};
use futures::sync::oneshot::spawn;
use humansize::{file_size_opts as options, FileSize};
use rusoto_lambda::{
    FunctionConfiguration, Lambda, LambdaClient, ListFunctionsError, ListFunctionsRequest,
};
use rusoto_resourcegroupstaggingapi::{
    GetResourcesError, GetResourcesInput, ResourceGroupsTaggingApi, ResourceGroupsTaggingApiClient,
    ResourceTagMapping, Tag,
};
use structopt::StructOpt;
use tokio::runtime::Runtime;

lazy_static! {
    static ref FALLBACK_RUNTIME: Runtime = Runtime::new().unwrap();
}

/// Failure types
#[derive(Fail, Debug)]
pub enum Error {
    #[fail(display = "{}", _0)]
    Listing(#[cause] ListFunctionsError),
    #[fail(display = "{}", _0)]
    Tags(#[cause] GetResourcesError),
}

impl From<ListFunctionsError> for Error {
    fn from(err: ListFunctionsError) -> Self {
        Error::Listing(err)
    }
}

impl From<GetResourcesError> for Error {
    fn from(err: GetResourcesError) -> Self {
        Error::Tags(err)
    }
}

/// CLI options
#[derive(StructOpt, PartialEq, Debug)]
#[structopt(name = "lambstock", about = "stock management for your AWS lambda")]
enum Options {
    #[structopt(name = "list", alias = "ls", about = "Lists lambdas")]
    List,
    #[structopt(name = "tags", about = "Lists lambdas tags")]
    Tags,
}

/// A single lambda function with associated tags
#[derive(Debug)]
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
) -> Box<Future<Item = Vec<ResourceTagMapping>, Error = GetResourcesError> + Send> {
    Box::new(
        client
            .get_resources(GetResourcesInput {
                resource_type_filters: Some(vec!["lambda:function".into()]),
                resources_per_page: Some(50),
                pagination_token,
                ..Default::default()
            })
            .and_then(move |result| {
                if let Some(token) = result.pagination_token.clone().filter(|s| !s.is_empty()) {
                    return future::Either::A(tag_mappings(client, Some(token)).map(|next| {
                        result
                            .resource_tag_mapping_list
                            .unwrap_or_default()
                            .into_iter()
                            .chain(next)
                            .collect()
                    }));
                }
                future::Either::B(future::ok(
                    result.resource_tag_mapping_list.unwrap_or_default(),
                ))
            }),
    )
}

fn main() -> Result<(), Error> {
    match Options::from_args() {
        Options::Tags => {
            let tags = tag_mappings(
                ResourceGroupsTaggingApiClient::new(Default::default()),
                Default::default(),
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
        Options::List => {
            let tags = tag_mappings(
                ResourceGroupsTaggingApiClient::new(Default::default()),
                Default::default(),
            ).map_err(Error::from);
            let lambdas = lambdas(LambdaClient::new(Default::default()), Default::default())
                .map_err(Error::from);
            let filtered = tags.join(lambdas).map(|(tags, lambdas)| {
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
            Ok(println!(
                "{:#?}",
                spawn(filtered, &FALLBACK_RUNTIME.executor()).wait()?
            ))
        }
    }
}
