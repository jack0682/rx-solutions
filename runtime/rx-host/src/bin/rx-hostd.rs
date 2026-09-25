use rx_host::service::{self, config::Loaded};
#[tokio::main]
async fn main() -> service::Result<()> {
    if let Err(error) = run().await {
        if let Some(refusal) = rx_service_status::storage_ownership_refusal(error.as_ref()) {
            eprintln!("{}", serde_json::to_string(&refusal)?);
            return Err(refusal.condition.into());
        }
        return Err(error);
    }
    Ok(())
}
async fn run() -> service::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() == 5 && args[0] == "prepare-binding-change" {
        let plan = rx_package::policy::read(std::path::Path::new(&args[1]))?;
        let current = Loaded::read(std::path::Path::new(&args[2]))?;
        let proposed = Loaded::read(std::path::Path::new(&args[3]))?;
        let request = rx_domain::types::Id::new(&args[4])?;
        println!(
            "{}",
            serde_json::to_string(&service::maintenance::prepare(
                &plan, &current, &proposed, &request
            )?)?
        );
        return Ok(());
    }
    if args.len() == 3 && args[0] == "lookup-binding-preparation" {
        let current = Loaded::read(std::path::Path::new(&args[1]))?;
        let request = rx_domain::types::Id::new(&args[2])?;
        println!(
            "{}",
            serde_json::to_string(&service::maintenance::lookup(&current, &request)?)?
        );
        return Ok(());
    }
    if args.len() == 3 && args[0] == "cancel-binding-preparation" {
        let current = Loaded::read(std::path::Path::new(&args[1]))?;
        let request = rx_domain::types::Id::new(&args[2])?;
        println!(
            "{}",
            serde_json::to_string(&service::maintenance::cancel(&current, &request)?)?
        );
        return Ok(());
    }
    if args.len() == 4 && args[0] == "inspect-binding-change" {
        let plan: rx_process_contract::host_binding_plan::Plan =
            rx_package::policy::read(std::path::Path::new(&args[1]))?;
        let current = Loaded::read(std::path::Path::new(&args[2]))?;
        let proposed = Loaded::read(std::path::Path::new(&args[3]))?;
        println!(
            "{}",
            serde_json::to_string(&service::binding_change::inspect(
                &plan, &current, &proposed
            )?)?
        );
        return Ok(());
    }
    if args.as_slice() == ["drivers", "dynamixel"] {
        println!(
            "{}",
            serde_json::to_string(&rx_host::dynamixel::profile::descriptor())?
        );
        return Ok(());
    }
    if args.as_slice() == ["drivers", "jtc"] {
        println!(
            "{}",
            serde_json::to_string(&service::jtc_package::driver())?
        );
        return Ok(());
    }
    if args.as_slice() == ["drivers"] {
        println!(
            "{}",
            serde_json::to_string(&service::device_package::driver())?
        );
        return Ok(());
    }
    if args.len() != 2 || !matches!(args[0].as_str(), "inspect" | "init" | "run") {
        return Err("usage: rx-hostd drivers [jtc] | inspect|init|run CONFIG | inspect-binding-change PLAN CURRENT_CONFIG PROPOSED_CONFIG | prepare-binding-change PLAN CURRENT_CONFIG PROPOSED_CONFIG REQUEST_ID | cancel-binding-preparation|lookup-binding-preparation CURRENT_CONFIG REQUEST_ID".into());
    }
    let loaded = Loaded::read(std::path::Path::new(&args[1]))?;
    if args[0] == "inspect" {
        println!("{}", service::inspect(&loaded)?);
        return Ok(());
    }
    let clock = rx_host::service_clock::SystemClock::new()?;
    if args[0] == "init" {
        service::initialize_with(&loaded, clock, &service::Builtin)?;
        println!("Host installation initialized; no native operation started");
        return Ok(());
    }
    #[cfg(unix)]
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let shutdown = async move {
        #[cfg(unix)]
        tokio::select! {_=term.recv()=>{},_=tokio::signal::ctrl_c()=>{}};
        #[cfg(not(unix))]
        let _ = tokio::signal::ctrl_c().await;
    };
    service::run_with(loaded, clock, service::Builtin, shutdown).await
}
