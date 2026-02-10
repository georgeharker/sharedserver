use anyhow::Result;

pub fn execute(name: &str, count: usize) -> Result<()> {
    let logs = sharedserver::core::log::read_recent_invocations(name, count)?;

    if logs.is_empty() {
        println!("No invocations logged for server '{}'", name);
        return Ok(());
    }

    println!("Recent invocations for server '{}':\n", name);

    for log in logs {
        println!("[{}] {} {}", log.timestamp, log.command, log.args.join(" "));
        println!("  Result: {}", log.result);

        if let Some(error) = &log.error {
            println!("  Error: {}", error);
        }

        if let Some(metadata) = &log.metadata {
            println!("  Metadata: {}", serde_json::to_string_pretty(metadata)?);
        }

        println!();
    }

    Ok(())
}
