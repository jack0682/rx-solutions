use rx_host::service::{self, config::Loaded};
#[tokio::main]
async fn main() -> service::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
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
        return Err("usage: rx-hostd drivers [jtc] | inspect|init|run CONFIG".into());
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
