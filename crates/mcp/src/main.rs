use std::{
    io::{self, BufReader, Write},
    path::PathBuf,
};
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let mut endpoint = None;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--endpoint") => {
                endpoint = Some(PathBuf::from(args.next().ok_or("missing endpoint")?))
            }
            Some("--help") => {
                eprintln!("pbrt-ui-mcp --endpoint /path/control.sock");
                return Ok(());
            }
            _ => return Err("unknown argument".into()),
        }
    }
    let client = cli::Client::connect(&endpoint.ok_or("--endpoint is required")?)?;
    let mut bridge = mcp::Bridge::new(client);
    let mut input = BufReader::new(io::stdin().lock());
    let mut output = io::stdout().lock();
    loop {
        let message = match protocol::ndjson::read_message::<serde_json::Value>(&mut input) {
            Ok(Some(value)) => value,
            Ok(None) => break,
            Err(error) => {
                let value = serde_json::json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":error.to_string()}});
                protocol::ndjson::write_message(&mut output, &value)?;
                output.flush()?;
                continue;
            }
        };
        if let Some(response) = bridge.handle(message) {
            protocol::ndjson::write_message(&mut output, &response)?;
            output.flush()?;
        }
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("pbrt-ui-mcp: {error}");
        std::process::exit(1);
    }
}
