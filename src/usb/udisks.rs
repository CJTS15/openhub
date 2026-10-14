//! Safe eject through UDisks2: unmount, lock, then power off, the same steps
//! GNOME Files takes. Polkit lets the active local user do this without a
//! password.

use std::collections::{HashMap, HashSet};

use zbus::fdo::ObjectManagerProxy;
use zbus::proxy;
use zbus::zvariant::{OwnedValue, Value};

const SERVICE: &str = "org.freedesktop.UDisks2";

#[proxy(
    interface = "org.freedesktop.UDisks2.Filesystem",
    default_service = "org.freedesktop.UDisks2"
)]
trait Filesystem {
    fn unmount(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.UDisks2.Encrypted",
    default_service = "org.freedesktop.UDisks2"
)]
trait Encrypted {
    fn lock(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.UDisks2.Drive",
    default_service = "org.freedesktop.UDisks2"
)]
trait Drive {
    fn power_off(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<()>;
}

/// One UDisks2 block device, as far as ejecting needs to know.
#[derive(Debug, Default, Clone)]
struct Block {
    path: String,
    /// Such as `/dev/sdb1`.
    device: String,
    /// The drive object, or `/` for virtual blocks such as unlocked LUKS.
    drive: String,
    /// For an unlocked LUKS volume, the encrypted block it comes from.
    crypto_backing: String,
    mounted: bool,
    /// An encrypted block that is unlocked.
    unlocked: bool,
}

/// UDisks2 calls to make, in order.
#[derive(Debug, Default, PartialEq)]
struct Plan {
    unmount: Vec<String>,
    lock: Vec<String>,
    power_off: Option<String>,
}

/// Ejects the drives behind `disks` (sysfs names such as `sdb`).
pub async fn eject(disks: &[String]) -> zbus::Result<()> {
    let conn = zbus::Connection::system().await?;
    let (blocks, powerable) = snapshot(&conn).await?;
    let plan = plan(&blocks, &powerable, disks);
    if plan == Plan::default() {
        return Err(zbus::Error::Failure(
            "UDisks2 doesn't know this drive".into(),
        ));
    }
    for path in &plan.unmount {
        let fs = FilesystemProxy::builder(&conn)
            .path(path.as_str())?
            .build()
            .await?;
        fs.unmount(HashMap::new()).await?;
    }
    for path in &plan.lock {
        let encrypted = EncryptedProxy::builder(&conn)
            .path(path.as_str())?
            .build()
            .await?;
        encrypted.lock(HashMap::new()).await?;
    }
    if let Some(path) = &plan.power_off {
        let drive = DriveProxy::builder(&conn)
            .path(path.as_str())?
            .build()
            .await?;
        drive.power_off(HashMap::new()).await?;
    }
    Ok(())
}

/// A file on the drive is still open, so it can't be unmounted.
pub fn is_busy(err: &zbus::Error) -> bool {
    matches!(err, zbus::Error::MethodError(name, _, _)
        if name.as_str() == "org.freedesktop.UDisks2.Error.DeviceBusy")
}

/// Every block device, and the drives that can be powered off.
async fn snapshot(conn: &zbus::Connection) -> zbus::Result<(Vec<Block>, HashSet<String>)> {
    let manager = ObjectManagerProxy::builder(conn)
        .destination(SERVICE)?
        .path("/org/freedesktop/UDisks2")?
        .build()
        .await?;
    let mut blocks = Vec::new();
    let mut powerable = HashSet::new();
    for (path, interfaces) in manager.get_managed_objects().await? {
        let mut block = Block {
            path: path.to_string(),
            ..Block::default()
        };
        let mut is_block = false;
        for (interface, props) in &interfaces {
            match interface.as_str() {
                "org.freedesktop.UDisks2.Drive" => {
                    if bool_prop(props, "CanPowerOff") {
                        powerable.insert(path.to_string());
                    }
                }
                "org.freedesktop.UDisks2.Block" => {
                    is_block = true;
                    block.device = bytes_prop(props, "Device");
                    block.drive = path_prop(props, "Drive");
                    block.crypto_backing = path_prop(props, "CryptoBackingDevice");
                }
                "org.freedesktop.UDisks2.Filesystem" => {
                    block.mounted = matches!(props.get("MountPoints").map(|v| &**v),
                        Some(Value::Array(points)) if !points.is_empty());
                }
                "org.freedesktop.UDisks2.Encrypted" => {
                    block.unlocked = path_prop(props, "CleartextDevice") != "/";
                }
                _ => {}
            }
        }
        if is_block {
            blocks.push(block);
        }
    }
    Ok((blocks, powerable))
}

/// What to call to eject the drives behind `disks`: unmount everything
/// mounted from them (unlocked LUKS volumes first), lock what's unlocked,
/// then power off when the drive supports it. Sibling drives of one USB
/// device, such as card reader slots, power off together.
fn plan(blocks: &[Block], powerable: &HashSet<String>, disks: &[String]) -> Plan {
    let devices: Vec<String> = disks.iter().map(|disk| format!("/dev/{disk}")).collect();
    let mut drives: Vec<&str> = blocks
        .iter()
        .filter(|block| devices.contains(&block.device) && block.drive != "/")
        .map(|block| block.drive.as_str())
        .collect();
    drives.dedup();
    let on_drives: Vec<&Block> = blocks
        .iter()
        .filter(|block| drives.contains(&block.drive.as_str()))
        .collect();
    let cleartext = blocks
        .iter()
        .filter(|block| on_drives.iter().any(|b| b.path == block.crypto_backing));
    Plan {
        unmount: cleartext
            .chain(on_drives.iter().copied())
            .filter(|block| block.mounted)
            .map(|block| block.path.clone())
            .collect(),
        lock: on_drives
            .iter()
            .filter(|block| block.unlocked)
            .map(|block| block.path.clone())
            .collect(),
        power_off: drives
            .into_iter()
            .find(|drive| powerable.contains(*drive))
            .map(str::to_owned),
    }
}

fn bool_prop(props: &HashMap<String, OwnedValue>, name: &str) -> bool {
    matches!(props.get(name).map(|v| &**v), Some(Value::Bool(true)))
}

fn path_prop(props: &HashMap<String, OwnedValue>, name: &str) -> String {
    match props.get(name).map(|v| &**v) {
        Some(Value::ObjectPath(path)) => path.to_string(),
        _ => "/".to_owned(),
    }
}

/// A NUL-terminated byte string (`ay`), as UDisks2 sends device paths.
fn bytes_prop(props: &HashMap<String, OwnedValue>, name: &str) -> String {
    let Some(Value::Array(array)) = props.get(name).map(|v| &**v) else {
        return String::new();
    };
    let bytes: Vec<u8> = array
        .iter()
        .filter_map(|byte| match byte {
            Value::U8(byte) => Some(*byte),
            _ => None,
        })
        .collect();
    String::from_utf8_lossy(&bytes)
        .trim_end_matches('\0')
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRIVE: &str = "/org/freedesktop/UDisks2/drives/SanDisk";

    fn block(name: &str, drive: &str) -> Block {
        Block {
            path: format!("/org/freedesktop/UDisks2/block_devices/{name}"),
            device: format!("/dev/{name}"),
            drive: drive.to_owned(),
            crypto_backing: "/".to_owned(),
            ..Block::default()
        }
    }

    fn path(name: &str) -> String {
        format!("/org/freedesktop/UDisks2/block_devices/{name}")
    }

    #[test]
    fn unmounts_partitions_then_powers_off() {
        let mut part1 = block("sdb1", DRIVE);
        part1.mounted = true;
        let part2 = block("sdb2", DRIVE);
        let other = block("nvme0n1p1", "/org/freedesktop/UDisks2/drives/Nvme");
        let blocks = [block("sdb", DRIVE), part1, part2, other];
        let powerable = HashSet::from([DRIVE.to_owned()]);
        let plan = plan(&blocks, &powerable, &["sdb".to_owned()]);
        assert_eq!(
            plan,
            Plan {
                unmount: vec![path("sdb1")],
                lock: vec![],
                power_off: Some(DRIVE.to_owned()),
            }
        );
    }

    #[test]
    fn unmounts_and_locks_an_encrypted_partition() {
        let mut luks = block("sdb1", DRIVE);
        luks.unlocked = true;
        let mut cleartext = block("dm-0", "/");
        cleartext.crypto_backing = path("sdb1");
        cleartext.mounted = true;
        let blocks = [block("sdb", DRIVE), luks, cleartext];
        let plan = plan(&blocks, &HashSet::new(), &["sdb".to_owned()]);
        assert_eq!(
            plan,
            Plan {
                unmount: vec![path("dm-0")],
                lock: vec![path("sdb1")],
                // This drive can't power off; unmounting is enough.
                power_off: None,
            }
        );
    }

    #[test]
    fn unknown_disk_plans_nothing() {
        let blocks = [block("sdb", DRIVE)];
        let plan = plan(&blocks, &HashSet::new(), &["sdc".to_owned()]);
        assert_eq!(plan, Plan::default());
    }
}
