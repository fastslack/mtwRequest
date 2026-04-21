pub mod init;
pub mod install;
pub mod new;
pub mod publish;
pub mod run;
pub mod search;

pub(crate) type CliResult = Result<(), Box<dyn std::error::Error>>;
