//! What a recovery does to use the keys of a Samsung phone, found in what
//! its own partitions say. The files under tests/samsung/p3q are the init
//! scripts and hardware descriptions of the key services of an SM-G9980's
//! stock Android 11 vendor partition, and the script of the system's own
//! directory for hardware services, the programs they name standing in as
//! empty files, kept apart from the whole stock builds under
//! tests/described:
//! a vendor whose key service is still Keymaster 4, reached as a hardware
//! service of the older kind, on a phone without slots and so without a
//! boot control.

use skyward_programs::phone::{Partition, Phone, PARTITIONS};
use skyward_programs::secure::{self, Step};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn described(codename: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/samsung").join(codename)
}

/// Where a recovery has each partition mounted.
fn mounted(partition: &str) -> String {
    match partition {
        "vendor" | "odm" => format!("/{partition}"),
        other => format!("/mnt/{other}"),
    }
}

/// The S21 Ultra as its vendor partition describes it, on a recovery of its
/// own make or, `foreign`, of another.
fn p3q(foreign: bool) -> Phone {
    let at = described("p3q");
    let partitions = PARTITIONS.iter().map(|name| Partition { name: name.to_string(), mounted: mounted(name).into(), at: at.join(name) }).collect();
    let properties: HashMap<String, String> =
        [("ro.boot.hardware", "qcom"), ("ro.boot.product.vendor.sku", "lahaina")].iter().map(|(n, v)| (n.to_string(), v.to_string())).collect();
    let mut phone = Phone::read(partitions, properties, None).unwrap();
    phone.foreign = foreign;
    phone
}

/// Each step as the name of the program's file it starts.
fn names(steps: &[Step]) -> Vec<String> {
    steps
        .iter()
        .map(|step| match step {
            Step::Start(started) => started.service.path.rsplit('/').next().unwrap().to_string(),
            Step::Drive(driver) => format!("(load {})", driver.module),
            Step::ServeElements => "(the secure element service)".into(),
        })
        .collect()
}

#[test]
fn the_s21_ultra_starts_the_key_services_of_the_older_kind_its_scripts_name() {
    let steps = secure::steps(&p3q(true)).unwrap();
    assert_eq!(
        names(&steps),
        ["qseecomd", "ssgtzd", "android.hardware.keymaster@4.0-service", "android.hardware.gatekeeper@1.0-service"],
        "the listeners, the opener, the key service and the credential service, and no boot control on a phone without slots; \
         not the system's own directory, beside the one the recovery of another make runs, through a loader of its own"
    );
    let Step::Start(keymaster) = &steps[2] else { panic!("the key service is started") };
    assert_eq!(keymaster.service.ready_hardware_service.as_deref(), Some("android.hardware.keymaster@4.0::IKeymasterDevice/default"));
    assert_eq!((keymaster.service.user.as_deref(), keymaster.service.groups.as_slice()), (Some("system"), &["system".to_string(), "drmrpc".to_string()][..]));
    assert!(keymaster.service.registers.is_empty(), "a hardware service registers only itself");
    let Step::Start(gatekeeper) = &steps[3] else { panic!("the credential service is started") };
    assert_eq!(gatekeeper.service.ready_hardware_service.as_deref(), Some("android.hardware.gatekeeper@1.0::IGatekeeper/default"));
    assert_eq!(gatekeeper.service.groups.last().map(String::as_str), Some("5678"), "a group the phone names by its number");
}

#[test]
fn on_a_recovery_of_another_make_a_hardware_service_runs_on_its_own_libraries_first() {
    let steps = secure::steps(&p3q(true)).unwrap();
    let Step::Start(keymaster) = &steps[2] else { panic!("the key service is started") };
    assert_eq!(keymaster.service.libraries[..3], ["/vendor/lib64", "/vendor/lib64/hw", "/system/lib64"].map(String::from));
    assert_eq!(keymaster.environment, [("LD_PRELOAD".to_string(), "/system/lib64/libc++.so".to_string())]);
    let Step::Start(listeners) = &steps[0] else { panic!("the listeners are started first") };
    assert_eq!(listeners.service.libraries[0], "/system/lib64", "a program that is no hardware service runs as it always did");
    assert!(listeners.environment.is_empty());
}

#[test]
fn on_its_own_recovery_a_hardware_service_runs_as_on_any_other_phone() {
    let steps = secure::steps(&p3q(false)).unwrap();
    let names = names(&steps);
    let directory = names.iter().position(|n| n == "hwservicemanager").expect("the system's own directory, as on any phone");
    let keys = names.iter().position(|n| n == "android.hardware.keymaster@4.0-service").unwrap();
    assert!(directory < keys, "the directory before the key service that registers with it: {names:?}");
    let Step::Start(keymaster) = &steps[keys] else { panic!("the key service is started") };
    assert!(keymaster.environment.is_empty(), "nothing loaded ahead on the recovery the libraries were built for");
}

#[test]
fn the_secure_world_s_opener_is_started_once_its_listeners_are_ready() {
    let steps = secure::steps(&p3q(true)).unwrap();
    let Step::Start(listeners) = &steps[0] else { panic!("the listeners are started first") };
    assert_eq!(listeners.service.ready_property.as_deref(), Some("vendor.sys.listeners.registered"));
    let Step::Start(opener) = &steps[1] else { panic!("the opener follows the listeners") };
    assert_eq!(opener.service.path, "/vendor/bin/ssgtzd");
    assert_eq!(opener.sockets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["ssgtzd", "qwes_ipc"]);
}

