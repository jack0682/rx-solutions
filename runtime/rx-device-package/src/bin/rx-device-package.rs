use rx_device_package::{directory, *};
use rx_domain::{canonical, types::*};
use rx_package::{SignatureEnvelope, policy};
use std::{fs::OpenOptions, io::Write, path::Path};
type AnyResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
fn output(value: serde_json::Value) -> AnyResult<()> {
    println!("{}", serde_json::to_string(&value)?);
    Ok(())
}
fn run() -> AnyResult<()> {
    let a = std::env::args().skip(1).collect::<Vec<_>>();
    match a.first().map(String::as_str) {
        Some("validator-identity") if a.len()==1=>output(serde_json::json!({"validator_digest":review::validator_digest(),"scope":"DEVICE_PACKAGE_SOFTWARE"})),
        Some("review") if a.len()==5=>{
            let request:rx_process_contract::device_review::Request=policy::read(Path::new(&a[3]))?;
            let (input,pin)=policy::read_with_digest::<policy::Policy>(Path::new(&a[2]))?;let policy=source_policy(&input)?;
            let mut bounded=policy.clone();bounded.max_files=bounded.max_files.min(8);bounded.max_content_bytes=bounded.max_content_bytes.min(2*1024*1024);
            let package=rx_package::directory::verify_directory(Path::new(&a[1]),&bounded)?;
            let report=review::verify(request,&package,&policy,pin)?;
            rx_package::directory::publish_files([(rx_package::PackagePath::new("verification.json")?,canonical::bytes(&report)?)].into(),Path::new(&a[4]))?;
            output(serde_json::json!({"status":"UNSIGNED_DEVICE_SOFTWARE_REPORT","report_digest":report.digest()?,"software_checks_passed":report.passed(),"physical_validation":"NOT_PERFORMED","activation_authorized":false}))
        },
        Some("review-signing-request") if a.len()==4=>{
            let report:rx_process_contract::device_review::Report=policy::read(Path::new(&a[1]))?;let key=Name::new(&a[2])?;let message=report.signing_message(&key)?;
            let request=serde_json::json!({"schema":"rx.device-report-signing-request.v1","key":key,"report_digest":report.digest()?,"message_digest":rx_package::content_digest(&message),"message_hex":message.iter().map(|b|format!("{b:02x}")).collect::<String>()});
            let mut f=OpenOptions::new().write(true).create_new(true).open(&a[3])?;f.write_all(&canonical::bytes(&request)?)?;f.sync_all()?;output(serde_json::json!({"status":"DEVICE_REPORT_SIGNATURE_REQUIRED","report_digest":report.digest()?}))
        },
        Some("template-digest") if a.len()==2=>{
            let value:serde_json::Value=policy::read(Path::new(&a[1]))?;
            let (digest,id,revision)=if value["schema"]=="rx.ros-jtc-template.v1" {
                let t:jtc::Template=serde_json::from_value(value)?;(t.digest()?,t.id,t.revision)
            } else {let t:Template=serde_json::from_value(value)?;(t.digest()?,t.id,t.revision)};
            output(serde_json::json!({"status":"TEMPLATE_STRUCTURE_VALID","template_digest":digest,"template":id,"revision":revision,"activation_authorized":false}))
        },
        Some("driver-identity") if a.len()==1=>output(serde_json::to_value(rx_host::service::device_package::driver())?),
        Some("driver-identity") if a.as_slice()==["driver-identity","jtc"]=>output(serde_json::to_value(rx_host::service::jtc_package::driver())?),
        Some("assemble") if a.len()==5=>{
            let value:serde_json::Value=policy::read(Path::new(&a[1]))?;let recipe:Recipe=policy::read(Path::new(&a[3]))?;
            let (candidate,digest)=if value["schema"]=="rx.ros-jtc-template.v1" {
                let t:jtc::Template=serde_json::from_value(value)?;let s:jtc::Site=policy::read(Path::new(&a[2]))?;
                (jtc::assemble(&t,&s,&recipe)?,t.digest()?)
            } else {let t:Template=serde_json::from_value(value)?;let s:Site=policy::read(Path::new(&a[2]))?;(assemble(&t,&s,&recipe)?,t.digest()?)};
            directory::publish(&candidate,None,Path::new(&a[4]))?;
            output(serde_json::json!({"status":"UNSIGNED_CANDIDATE","manifest_digest":candidate.digest()?,"template_digest":digest,"activation_authorized":false}))
        },
        Some("request") if a.len()==4=>{
            let candidate=directory::candidate(Path::new(&a[1]))?;let key=Name::new(&a[2])?;let message=candidate.signing_message(&key)?;
            let request=serde_json::json!({"schema":"rx.package-signing-request.v1","key":key,"manifest_digest":candidate.digest()?,"message_digest":rx_package::content_digest(&message),"message_hex":message.iter().map(|b|format!("{b:02x}")).collect::<String>()});
            let mut file=OpenOptions::new().write(true).create_new(true).open(&a[3])?;file.write_all(&canonical::bytes(&request)?)?;file.sync_all()?;
            output(serde_json::json!({"status":"SIGNATURE_REQUIRED","manifest_digest":candidate.digest()?}))
        },
        Some("seal") if a.len()==5=>{
            let candidate=directory::candidate(Path::new(&a[1]))?;let signature:SignatureEnvelope=policy::read(Path::new(&a[2]))?;let input:policy::Policy=policy::read(Path::new(&a[3]))?;
            let package=candidate.verify(&signature,&load_policy(&input)?)?;directory::publish(&candidate,Some(&signature),Path::new(&a[4]))?;
            output(serde_json::json!({"status":"CONTENT_VERIFIED_NOT_QUALIFIED","manifest_digest":package.digest(),"activation_authorized":false}))
        },
        Some("verify")|Some("inspect") if a.len()==3=>{
            let input:policy::Policy=policy::read(Path::new(&a[2]))?;let package=rx_package::directory::verify_directory(Path::new(&a[1]),&load_policy(&input)?)?;
            let mut value=match decode_verified_any(&package)? {
                Device::Melsec(device)=>{
                    let mut v=serde_json::json!({"profile_digest":device.profile.digest()?,"installation":device.profile.installation,"cell":device.profile.cell,"environment":device.profile.environment()});
                    if a[0]=="inspect"{v["profile"]=serde_json::to_value(device.profile)?;}v
                },
                Device::Jtc(device)=>{
                    let mut v=serde_json::json!({"profile_digest":device.profile.digest()?,"installation":device.profile.installation,"cell":device.profile.cell,"environment":device.profile.environment,"control_provider":"NOT_CONFIGURED"});
                    if a[0]=="inspect"{v["profile"]=serde_json::to_value(device.profile)?;v["operations"]=serde_json::to_value(device.operations)?;v["outcomes"]=serde_json::to_value(device.outcomes)?;}v
                },
            };
            value["status"]=serde_json::json!("CONTENT_VERIFIED_NOT_QUALIFIED");value["manifest_digest"]=serde_json::to_value(package.digest())?;value["activation_authorized"]=serde_json::json!(false);
            output(value)
        },
        _=>Err("usage: rx-device-package validator-identity | review PACKAGE POLICY REQUEST OUT | review-signing-request REPORT KEY_ID OUT_FILE | driver-identity [jtc] | template-digest TEMPLATE | assemble TEMPLATE SITE RECIPE OUT | request CANDIDATE KEY_ID OUT_FILE | seal CANDIDATE SIGNATURE POLICY OUT | verify PACKAGE POLICY | inspect PACKAGE POLICY".into()),
    }
}
fn main() {
    if let Err(e) = run() {
        eprintln!("rx-device-package: {e}");
        std::process::exit(1);
    }
}
