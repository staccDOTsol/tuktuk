use solana_sdk::message::CompileError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("RPC error: {0}")]
    RpcError(#[from] solana_client::client_error::ClientError),
    #[error("Instruction error: {0}")]
    InstructionError(#[from] solana_sdk::instruction::InstructionError),
    #[error("Compile error: {0}")]
    CompileError(#[from] CompileError),
}
