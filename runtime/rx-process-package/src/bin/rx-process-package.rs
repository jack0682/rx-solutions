use rx_domain::{canonical, types::*};
use rx_package::{PackagePath, SignatureEnvelope};
use rx_process_contract::compile_input::CompileInput;
use rx_process_package::{Recipe, directory, trust};
use std::{collections::BTreeMap, fs::OpenOptions, io::Write, path::Path};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
fn output_json(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("rx-process-package: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("validator-identity") if args.len()==1=>output_json(&serde_json::json!({"validator_digest":rx_process_package::review::validator_digest(),"scope":"PROCESS_PACKAGE_SOFTWARE"})),
        Some("review") if args.len()==5=>{
            let request:rx_process_contract::package_review::Request=trust::read(Path::new(&args[3]))?;
            let (policy_doc,file_digest)=rx_package::policy::read_with_digest::<trust::Policy>(Path::new(&args[2]))?;
            let policy=policy_doc.load()?;let package=rx_package::directory::verify_directory(Path::new(&args[1]),&policy)?;
            let bundle=rx_process_package::review::verify_process(request,&package,&policy,file_digest)?;
            let report_digest=bundle.report.digest()?;let ready=bundle.report.issues.is_empty();
            directory::publish_files(bundle.files,Path::new(&args[4]))?;
            output_json(&serde_json::json!({"status":"UNSIGNED_VERIFICATION_REPORT","report_digest":report_digest,"compiler_checks_passed":ready,"activation_authorized":false}))
        },
        Some("review-signing-request") if args.len()==4=>{
            let report:rx_process_contract::package_review::Report=trust::read(Path::new(&args[1]))?;
            let key=Name::new(&args[2])?;let message=report.signing_message(&key)?;
            let value=serde_json::json!({"schema":"rx.verification-signing-request.v1","key":key,"report_digest":report.digest()?,"message_digest":rx_package::content_digest(&message),"message_hex":message.iter().map(|b|format!("{b:02x}")).collect::<String>()});
            let mut f=OpenOptions::new().write(true).create_new(true).open(&args[3])?;f.write_all(&canonical::bytes(&value)?)?;f.sync_all()?;output_json(&serde_json::json!({"status":"SIGNATURE_REQUIRED","report_digest":report.digest()?}))
        },
        Some("assemble") if args.len()==4=>{
            let input:CompileInput=trust::read(Path::new(&args[1]))?;let recipe:Recipe=trust::read(Path::new(&args[2]))?;let candidate=rx_process_package::assemble(&input,&recipe)?;
            directory::publish(&candidate,None,Path::new(&args[3]))?;output_json(&serde_json::json!({"status":"UNSIGNED_CANDIDATE","manifest_digest":candidate.digest()?,"permissions":candidate.manifest().permissions}))
        },
        Some("request") if args.len()==4=>{
            let candidate=directory::candidate(Path::new(&args[1]))?;let key=Name::new(&args[2])?;let message=candidate.signing_message(&key)?;
            let request=serde_json::json!({"schema":"rx.package-signing-request.v1","key":key,"manifest_digest":candidate.digest()?,"message_digest":rx_package::content_digest(&message),"message_hex":message.iter().map(|b|format!("{b:02x}")).collect::<String>()});
            let mut file=OpenOptions::new().write(true).create_new(true).open(&args[3])?;file.write_all(&canonical::bytes(&request)?)?;file.sync_all()?;output_json(&serde_json::json!({"status":"SIGNATURE_REQUIRED","manifest_digest":candidate.digest()?}))
        },
        Some("seal") if args.len()==5=>{
            let candidate=directory::candidate(Path::new(&args[1]))?;let signature:SignatureEnvelope=trust::read(Path::new(&args[2]))?;let policy:trust::Policy=trust::read(Path::new(&args[3]))?;
            let verified=candidate.verify(&signature,&policy.load()?)?;rx_process_package::compile_verified(&verified)?;
            directory::publish(&candidate,Some(&signature),Path::new(&args[4]))?;output_json(&serde_json::json!({"status":"CONTENT_VERIFIED_NOT_QUALIFIED","manifest_digest":verified.digest()}))
        },
        Some("verify")|Some("compile") if args.len()==3||args.len()==4=>{
            let compiling=args[0]=="compile";if args.len()!=if compiling {4}else{3}{return Err("invalid verify/compile arguments".into());}
            let policy:trust::Policy=trust::read(Path::new(&args[2]))?;let package=rx_package::directory::verify_directory(Path::new(&args[1]),&policy.load()?)?;let process=rx_process_package::compile_verified(&package)?;
            if compiling {
                let files:BTreeMap<_,_>=[(PackagePath::new("resolved.json")?,canonical::bytes(&process)?),(PackagePath::new("process.bt.xml")?,rx_process::bt_xml::generate(&process)?.into_bytes())].into_iter().collect();directory::publish_files(files,Path::new(&args[3]))?;
            }
            output_json(&serde_json::json!({"status":"CONTENT_VERIFIED_NOT_QUALIFIED","manifest_digest":package.digest(),"process":process.process,"package_digest":process.package_digest}))
        },
        _=>Err("usage: rx-process-package validator-identity | review PACKAGE POLICY REQUEST OUT | review-signing-request REPORT KEY_ID OUT_FILE | assemble BUNDLE RECIPE OUT | request CANDIDATE KEY_ID OUT_FILE | seal CANDIDATE SIGNATURE POLICY OUT | verify PACKAGE POLICY | compile PACKAGE POLICY OUT".into()),
    }
}
