use std::io;

pub mod pb {
    tonic::include_proto!("tunnel");
}

#[tokio::main]
async fn main() -> io::Result<()> {
    Ok(())
}
