//! Outcomes distinguished by the existing Admin mutation contracts.
#[derive(Debug)]
pub(crate) enum WriteError {
    Sql(sqlx::Error),
    Mutation(sqlx::Error),
    Unavailable,
    NotFound,
    Invalid(&'static str),
    Gone(&'static str),
    Missing(&'static str),
    UnavailableCode(&'static str),
    Conflict(&'static str),
}
impl From<sqlx::Error> for WriteError {
    fn from(error: sqlx::Error) -> Self {
        Self::Sql(error)
    }
}
