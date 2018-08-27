use rusoto_lambda::ListFunctionsError;
use rusoto_resourcegroupstaggingapi::GetResourcesError;

/// Failure types
#[derive(Fail, Debug)]
pub enum Error {
    #[fail(display = "{}", _0)]
    Listing(
        #[cause]
        ListFunctionsError
    ),
    #[fail(display = "{}", _0)]
    Tags(
        #[cause]
        GetResourcesError
    ),
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
