use failure::Fail;
use rusoto_core::RusotoError;
use rusoto_lambda::ListFunctionsError;
use rusoto_resourcegroupstaggingapi::GetResourcesError;

/// Failure types
#[derive(Fail, Debug)]
pub enum Error {
    #[fail(display = "{}", _0)]
    Listing(#[cause] RusotoError<ListFunctionsError>),
    #[fail(display = "{}", _0)]
    Tags(#[cause] RusotoError<GetResourcesError>),
}

impl From<RusotoError<ListFunctionsError>> for Error {
    fn from(err: RusotoError<ListFunctionsError>) -> Self {
        Error::Listing(err)
    }
}

impl From<RusotoError<GetResourcesError>> for Error {
    fn from(err: RusotoError<GetResourcesError>) -> Self {
        Error::Tags(err)
    }
}
