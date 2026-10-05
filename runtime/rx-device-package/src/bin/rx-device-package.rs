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
        #[cfg(unix)]
        Some("external-sdk") if a.len()==2=>{
            let files=std::collections::BTreeMap::from([(rx_package::PackagePath::new("rx_external_adapter.py")?,include_bytes!("../../../../deployment/external-adapters/rx_external_adapter.py").to_vec())]);
            rx_package::directory::publish_files(files,Path::new(&a[1]))?;
            output(serde_json::json!({"status":"EXTERNAL_ADAPTER_SDK_EXPORTED","protocol":"rx.external-process-channel.v1","activation_authorized":false}))
        },
        #[cfg(unix)]
        Some("external-program") if a.len()==5=>{
            use rx_host::external_process::profile::{FilePin,Program,PROGRAM_SCHEMA};
            let pin=|p:std::path::PathBuf|->AnyResult<FilePin>{
                let meta=std::fs::symlink_metadata(&p)?;
                if !p.is_absolute() || !meta.is_file() || meta.file_type().is_symlink() || meta.len()==0 || meta.len()>16*1024*1024 {return Err("regular bounded absolute program file required".into());}
                Ok(FilePin {sha256:rx_package::content_digest(&std::fs::read(&p)?),size_bytes:Counter(meta.len()),path:p})
            };
            let dependencies:Vec<std::path::PathBuf>=policy::read(Path::new(&a[3]))?;
            let program=Program {schema:Name::new(PROGRAM_SCHEMA)?,executable:pin((&a[1]).into())?,arguments:policy::read(Path::new(&a[2]))?,dependencies:dependencies.into_iter().map(pin).collect::<AnyResult<_>>()?};
            program.validate(true)?;let mut file=OpenOptions::new().write(true).create_new(true).open(&a[4])?;
            file.write_all(&canonical::bytes(&program)?)?;file.sync_all()?;
            output(serde_json::json!({"status":"PINNED_EXTERNAL_PROGRAM","reference":program.reference()?,"activation_authorized":false}))
        },
        #[cfg(unix)]
        Some("external-assemble") if a.len()==4=>{
            let source=policy::read(Path::new(&a[1]))?;let recipe:Recipe=policy::read(Path::new(&a[2]))?;
            let candidate=external::assemble(&source,&recipe)?;directory::publish(&candidate,None,Path::new(&a[3]))?;
            output(serde_json::json!({"status":"UNSIGNED_CANDIDATE","manifest_digest":candidate.digest()?,"activation_authorized":false}))
        },
        #[cfg(unix)]
        Some("external-register") if a.len()==5 || a.len()==6=>{
            use rx_host::service::external_package::{Registry,Registration};
            let directory=std::path::PathBuf::from(&a[1]);let policy_path=std::path::PathBuf::from(&a[2]);
            if !directory.is_absolute() || !policy_path.is_absolute() {return Err("absolute installed package/policy paths required".into());}
            let input:policy::Policy=policy::read(&policy_path)?;
            let package=rx_package::directory::verify_directory(&directory,&load_policy(&input)?)?;
            let Device::External(checked)=decode_verified_any(&package)? else{return Err("external provider package required".into());};
            checked.program.validate(true)?;
            let mut registry=if a.len()==6 {policy::read::<Registry>(Path::new(&a[5]))?} else {Registry {schema:Name::new("rx.external-adapter-registry.v1")?,entries:Default::default()}};
            let key=Name::new(&a[3])?;
            if registry.schema.as_str()!="rx.external-adapter-registry.v1" || registry.entries.len()>=64 || registry.entries.contains_key(&key) {return Err("registry schema/bound or existing registration; replacement not performed".into());}
            registry.entries.insert(key.clone(),Registration {directory,manifest_digest:package.digest(),policy:rx_host::service::config::PinnedFile {path:policy_path.clone(),sha256:rx_package::content_digest(&std::fs::read(&policy_path)?)}});
            let raw=canonical::bytes(&registry)?;let mut file=OpenOptions::new().write(true).create_new(true).open(&a[4])?;file.write_all(&raw)?;file.sync_all()?;
            output(serde_json::json!({"status":"REGISTERED_AVAILABILITY_ONLY","adapter":key,"registry_sha256":rx_package::content_digest(&raw),"manifest_digest":package.digest(),"activation_authorized":false,"native_processes_started":0}))
        },
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
        #[cfg(unix)]
        Some("python-execution-assemble") if a.len()==5=>{
            let source=policy::read(Path::new(&a[1]))?;
            let environment=std::fs::read(&a[2])?;let recipe:Recipe=policy::read(Path::new(&a[3]))?;
            let candidate=python::assemble_execution(&source,&environment,&recipe)?;
            directory::publish(&candidate,None,Path::new(&a[4]))?;
            output(serde_json::json!({"status":"UNSIGNED_CANDIDATE","manifest_digest":candidate.digest()?,"activation_authorized":false}))
        },
        Some("python-library-assemble") if a.len()==5=>{
            let library:rx_host::service::python_library::Library=policy::read(Path::new(&a[1]))?;
            let environment=std::fs::read(&a[2])?;let recipe:Recipe=policy::read(Path::new(&a[3]))?;
            let candidate=python::assemble_library(&library,&environment,&recipe)?;
            directory::publish(&candidate,None,Path::new(&a[4]))?;
            output(serde_json::json!({"status":"UNSIGNED_CANDIDATE","manifest_digest":candidate.digest()?,"activation_authorized":false}))
        },
        #[cfg(unix)]
        Some("python-assemble") if a.len()==5=>{
            let registration:rx_host::service::python_skill::Registration=policy::read(Path::new(&a[1]))?;
            let environment=std::fs::read(&a[2])?;let recipe:Recipe=policy::read(Path::new(&a[3]))?;
            let candidate=python::assemble(&registration,&environment,&recipe)?;
            directory::publish(&candidate,None,Path::new(&a[4]))?;
            output(serde_json::json!({"status":"UNSIGNED_CANDIDATE","manifest_digest":candidate.digest()?,"activation_authorized":false}))
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
                #[cfg(unix)]
                Device::External(value)=>serde_json::json!({"profile":value.profile,"templates":value.templates.catalog(),"environment":"SIMULATION","physical_qualification":false}),
                #[cfg(unix)]
                Device::PythonExecution(value)=>serde_json::json!({"profile":value.profile,"templates":value.templates.catalog(),"environment":"SIMULATION","physical_qualification":false}),
                #[cfg(unix)]
                Device::PythonLibrary(profile)=>serde_json::json!({"profile_digest":profile.profile_digest()?,"installation":profile.installation,"cell":profile.cell,"environment":"SIMULATION","profile":profile,"physical_qualification":false}),
                #[cfg(unix)]
                Device::Python(profile)=>serde_json::json!({"profile_digest":profile.intent.profile_digest,"installation":profile.installation,"cell":profile.cell,"environment":"SIMULATION","profile":profile,"physical_qualification":false}),

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
        _=>Err("usage: rx-device-package external-sdk OUT | external-program EXECUTABLE ARGUMENTS_JSON DEPENDENCIES_JSON OUT | external-assemble ASSEMBLY RECIPE OUT | external-register PACKAGE POLICY KEY OUT [BASE_REGISTRY] | python-execution-assemble ASSEMBLY ENVIRONMENT RECIPE OUT | python-library-assemble LIBRARY ENVIRONMENT RECIPE OUT | python-assemble REGISTRATION ENVIRONMENT RECIPE OUT | validator-identity | review PACKAGE POLICY REQUEST OUT | review-signing-request REPORT KEY_ID OUT_FILE | driver-identity [jtc] | template-digest TEMPLATE | assemble TEMPLATE SITE RECIPE OUT | request CANDIDATE KEY_ID OUT_FILE | seal CANDIDATE SIGNATURE POLICY OUT | verify PACKAGE POLICY | inspect PACKAGE POLICY".into()),
    }
}
fn main() {
    if let Err(e) = run() {
        eprintln!("rx-device-package: {e}");
        std::process::exit(1);
    }
}
