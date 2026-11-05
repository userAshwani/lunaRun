//! Who is listening on a port (FR-SV-03, docs/SERVICES.md section 3).
//!
//! Two jobs. **Finding out that a port is taken:** a test bind of `127.0.0.1:<port>` is not enough, because on
//! Windows and macOS a bind to a specific address succeeds even while another program listens on the wildcard address
//! of the same port, which is exactly how XAMPP's `mysqld` listens. So the OS's own table of listening sockets is
//! asked too. **Naming the culprit:** the process that holds it, so the UI can say "XAMPP MySQL is running from
//! C:\xampp\mysql\bin\mysqld.exe" and offer another port.
//!
//! One call per OS, no crate for it (D-024): Windows `GetExtendedTcpTable` through the `windows-sys` bindings, Linux
//! `/proc/net/tcp` and `/proc/<pid>/fd`, macOS `lsof`. The parsers of the Linux and `lsof` text are plain functions
//! with their own tests, so they are checked on every OS.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::os;

/// A process that is listening on a port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PortOwner {
    pub pid: u32,
    /// `mysqld.exe`, from the program's path when the OS gives it, else from the OS's own short name.
    pub process_name: Option<String>,
    /// Where the program is, when the OS tells us (it does not for processes of another user).
    pub process_path: Option<String>,
}

/// A socket in the listening state that would clash with a bind to `127.0.0.1:<port>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Listener {
    pub port: u16,
    /// `None`: the OS shows the socket but not who owns it (another user's process), or it is not looked up yet.
    pub pid: Option<u32>,
    /// The OS's short name for the process, if it gave one.
    pub name: Option<String>,
    /// Linux: the socket's inode, from which the owner is found when it is wanted.
    pub inode: Option<u64>,
}

/// Would a listener on `addr` take the port away from a bind to `127.0.0.1`? The wildcard and the IPv4 loopback do;
/// the IPv6 loopback and an address of another network card do not.
pub(super) fn clashes_with_loopback(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => v4.is_unspecified() || v4.is_loopback(),
        // `::` accepts IPv4 connections too on most systems. `::ffff:127.0.0.1` is the same as 127.0.0.1.
        IpAddr::V6(v6) => {
            v6.is_unspecified()
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| v4.is_loopback() || v4.is_unspecified())
        }
    }
}

/// The ports that have a listener clashing with 127.0.0.1, found with one call to the OS. This is what the allocator
/// scans a range against.
pub(super) fn listening_ports() -> std::collections::HashSet<u16> {
    imp::all()
        .into_iter()
        .map(|listener| listener.port)
        .collect()
}

/// Everything that listens on `port` in a way that clashes with 127.0.0.1, whether or not its owner is known.
pub(super) fn conflicting_listeners(port: u16) -> Vec<Listener> {
    imp::all()
        .into_iter()
        .filter(|listener| listener.port == port)
        .collect()
}

/// The processes behind `listeners` (the ones whose owner the OS tells).
pub(super) fn owners_of(listeners: Vec<Listener>) -> Vec<PortOwner> {
    listeners
        .into_iter()
        .filter_map(|mut listener| {
            imp::find_owner(&mut listener);
            let pid = listener.pid?;
            let path = os::process_exe(pid);
            let name = path
                .as_deref()
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy().into_owned())
                .or(listener.name);
            Some(PortOwner {
                pid,
                process_name: name,
                process_path: path.map(|p| p.display().to_string()),
            })
        })
        .collect()
}

/// The processes that hold `port`. Empty when it is free, and also when it is held but the OS does not say by whom.
pub fn port_owners(port: u16) -> Vec<PortOwner> {
    owners_of(conflicting_listeners(port))
}

// ---------------------------------------------------------------------------------------------
// Parsers (used on Linux and macOS, tested everywhere)
// ---------------------------------------------------------------------------------------------

/// A line of `/proc/net/tcp` or `/proc/net/tcp6` in the listening state.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) struct ProcSocket {
    pub addr: IpAddr,
    pub port: u16,
    pub inode: u64,
}

/// Parse `/proc/net/tcp` (`v6 == false`) or `/proc/net/tcp6` and keep the sockets in the LISTEN state (`0A`).
///
/// ```text
///   sl  local_address rem_address   st tx_queue:rx_queue tr:tm->when retrnsmt   uid  timeout inode
///    0: 0100007F:0CEA 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 12345 1 ...
/// ```
///
/// Addresses are printed as the kernel's native-endian words of the network-order bytes, so on the little-endian
/// machines lunaRun runs on, the bytes of every 32-bit word come out in reverse.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) fn parse_proc_net_tcp(text: &str, v6: bool) -> Vec<ProcSocket> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let (_slot, local, _remote, state) = (
                fields.next()?,
                fields.next()?,
                fields.next()?,
                fields.next()?,
            );
            if state != "0A" {
                return None;
            }
            // uid, timeout and inode come after the queues, timers and retransmit count.
            let inode = fields.nth(5)?;
            let (host, port) = local.split_once(':')?;
            let port = u16::from_str_radix(port, 16).ok()?;
            let addr = if v6 {
                parse_v6_words(host)?
            } else {
                parse_v4_word(host)?
            };
            Some(ProcSocket {
                addr,
                port,
                inode: inode.parse().ok()?,
            })
        })
        .collect()
}

fn parse_v4_word(hex: &str) -> Option<IpAddr> {
    let word = u32::from_str_radix(hex, 16).ok()?;
    Some(IpAddr::V4(Ipv4Addr::from(word.to_le_bytes())))
}

fn parse_v6_words(hex: &str) -> Option<IpAddr> {
    if hex.len() != 32 {
        return None;
    }
    let mut bytes = [0_u8; 16];
    for (index, chunk) in bytes.chunks_mut(4).enumerate() {
        let word = u32::from_str_radix(hex.get(index * 8..index * 8 + 8)?, 16).ok()?;
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    Some(IpAddr::V6(Ipv6Addr::from(bytes)))
}

/// One listening socket from `lsof -F`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) struct LsofSocket {
    pub pid: u32,
    pub command: String,
    pub addr: IpAddr,
    pub port: u16,
}

/// Parse the output of `lsof -nP -iTCP -sTCP:LISTEN -Fpcn`: `p<pid>` starts a process, `c<command>` names it, and
/// every `n<address>:<port>` is one of its listening sockets (`*:3306`, `127.0.0.1:8080`, `[::1]:9000`).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) fn parse_lsof(text: &str) -> Vec<LsofSocket> {
    let mut out = Vec::new();
    let (mut pid, mut command) = (None, String::new());
    for line in text.lines() {
        let Some(kind) = line.chars().next() else {
            continue;
        };
        let rest = &line[kind.len_utf8()..];
        match kind {
            'p' => {
                pid = rest.parse::<u32>().ok();
                command.clear();
            }
            'c' => rest.clone_into(&mut command),
            'n' => {
                let Some(pid) = pid else { continue };
                let Some((host, port)) = rest.rsplit_once(':') else {
                    continue;
                };
                let Ok(port) = port.parse::<u16>() else {
                    continue;
                };
                let addr = match host {
                    "*" => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                    host => match host.trim_matches(['[', ']']).parse::<IpAddr>() {
                        Ok(addr) => addr,
                        Err(_) => continue,
                    },
                };
                out.push(LsofSocket {
                    pid,
                    command: command.clone(),
                    addr,
                    port,
                });
            }
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------------------------

#[cfg(windows)]
mod imp {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP6TABLE_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
        TCP_TABLE_OWNER_PID_LISTENER,
    };
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

    use super::{Listener, clashes_with_loopback};

    /// `ERROR_INSUFFICIENT_BUFFER`: the table is bigger than the buffer, and the size needed has been written back.
    const INSUFFICIENT_BUFFER: u32 = 122;

    /// The table of listening sockets, as 32-bit words (the rows must be 4-byte aligned).
    fn table(family: u32) -> Option<Vec<u32>> {
        let mut size = 0_u32;
        // The table can grow between the two calls, so ask again a few times.
        for _ in 0..5 {
            let mut buffer = vec![0_u32; (size as usize).div_ceil(4).max(1)];
            // SAFETY: the buffer holds at least `size` bytes and the call writes at most that many, updating `size`.
            let status = unsafe {
                GetExtendedTcpTable(
                    buffer.as_mut_ptr().cast(),
                    &mut size,
                    0,
                    family,
                    TCP_TABLE_OWNER_PID_LISTENER,
                    0,
                )
            };
            match status {
                0 => return Some(buffer),
                INSUFFICIENT_BUFFER => {}
                _ => return None,
            }
        }
        None
    }

    /// The port in a row is in network byte order, in the low 16 bits.
    fn host_port(raw: u32) -> u16 {
        u16::from_be((raw & 0xFFFF) as u16)
    }

    fn listener(port: u16, pid: u32) -> Listener {
        Listener {
            port,
            pid: Some(pid),
            name: None,
            inode: None,
        }
    }

    pub fn all() -> Vec<Listener> {
        let mut out = Vec::new();

        if let Some(buffer) = table(u32::from(AF_INET)) {
            let header = buffer.as_ptr().cast::<MIB_TCPTABLE_OWNER_PID>();
            // SAFETY: a successful call filled the buffer with a `MIB_TCPTABLE_OWNER_PID`: the count, then that many
            // rows. The buffer is 4-byte aligned and lives for the whole block.
            let rows = unsafe {
                std::slice::from_raw_parts(
                    (*header).table.as_ptr(),
                    (*header).dwNumEntries as usize,
                )
            };
            for row in rows {
                // The address is stored in network byte order.
                let addr = IpAddr::V4(Ipv4Addr::from(row.dwLocalAddr.to_le_bytes()));
                if clashes_with_loopback(addr) {
                    out.push(listener(host_port(row.dwLocalPort), row.dwOwningPid));
                }
            }
        }

        if let Some(buffer) = table(u32::from(AF_INET6)) {
            let header = buffer.as_ptr().cast::<MIB_TCP6TABLE_OWNER_PID>();
            // SAFETY: as above, for the IPv6 table.
            let rows = unsafe {
                std::slice::from_raw_parts(
                    (*header).table.as_ptr(),
                    (*header).dwNumEntries as usize,
                )
            };
            for row in rows {
                let addr = IpAddr::V6(Ipv6Addr::from(row.ucLocalAddr));
                if clashes_with_loopback(addr) {
                    out.push(listener(host_port(row.dwLocalPort), row.dwOwningPid));
                }
            }
        }
        out
    }

    /// The owner is already known.
    pub fn find_owner(_listener: &mut Listener) {}
}

// ---------------------------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod imp {
    use std::fs;

    use super::{Listener, clashes_with_loopback, parse_proc_net_tcp};

    pub fn all() -> Vec<Listener> {
        let mut out = Vec::new();
        for (file, v6) in [("/proc/net/tcp", false), ("/proc/net/tcp6", true)] {
            let Ok(text) = fs::read_to_string(file) else {
                continue;
            };
            out.extend(
                parse_proc_net_tcp(&text, v6)
                    .into_iter()
                    .filter(|socket| clashes_with_loopback(socket.addr))
                    .map(|socket| Listener {
                        port: socket.port,
                        pid: None,
                        name: None,
                        inode: Some(socket.inode),
                    }),
            );
        }
        out
    }

    /// Look the owner up now: only for a port that is actually taken, because it means reading the file descriptors
    /// of every process.
    pub fn find_owner(listener: &mut Listener) {
        let Some(inode) = listener.inode else {
            return;
        };
        listener.pid = pid_of_socket(inode);
        listener.name = listener.pid.and_then(command_name);
    }

    /// The process that has the socket open: look for `socket:[<inode>]` among every process's file descriptors.
    /// Processes of other users cannot be looked into, so a socket of theirs has no owner here.
    fn pid_of_socket(inode: u64) -> Option<u32> {
        let wanted = format!("socket:[{inode}]");
        for entry in fs::read_dir("/proc").ok()?.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            let Ok(fds) = fs::read_dir(entry.path().join("fd")) else {
                continue;
            };
            for fd in fds.flatten() {
                if fs::read_link(fd.path())
                    .is_ok_and(|target| target.as_os_str() == wanted.as_str())
                {
                    return Some(pid);
                }
            }
        }
        None
    }

    fn command_name(pid: u32) -> Option<String> {
        let name = fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
        Some(name.trim().to_owned())
    }
}

// ---------------------------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod imp {
    use std::process::{Command, Stdio};

    use super::{Listener, clashes_with_loopback, parse_lsof};

    /// The system's own `lsof`, by absolute path, with an argument array (never a shell string).
    const LSOF: &str = "/usr/sbin/lsof";

    pub fn all() -> Vec<Listener> {
        // One call lists every listening TCP socket the current user may see.
        let Ok(output) = Command::new(LSOF)
            .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpcn"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        else {
            return Vec::new();
        };
        parse_lsof(&String::from_utf8_lossy(&output.stdout))
            .into_iter()
            .filter(|socket| clashes_with_loopback(socket.addr))
            .map(|socket| Listener {
                port: socket.port,
                pid: Some(socket.pid),
                name: Some(socket.command),
                inode: None,
            })
            .collect()
    }

    /// The owner is already known.
    pub fn find_owner(_listener: &mut Listener) {}
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod imp {
    use super::Listener;

    pub fn all() -> Vec<Listener> {
        Vec::new()
    }

    pub fn find_owner(_listener: &mut Listener) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROC_TCP: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:0CEA 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 12345 1 0000000000000000 100 0 0 10 0
   1: 00000000:0050 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 22222 1 0000000000000000 100 0 0 10 0
   2: 0100007F:1F90 0100007F:D2F0 01 00000000:00000000 00:00000000 00000000  1000        0 33333 1 0000000000000000 20 4 30 10 -1
   3: 0500A8C0:0CEA 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 44444 1 0000000000000000 100 0 0 10 0
";

    const PROC_TCP6: &str = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000001000000:2328 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 55555 1 0000000000000000 100 0 0 10 0
   1: 00000000000000000000000000000000:0CEA 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 66666 1 0000000000000000 100 0 0 10 0
";

    #[test]
    fn the_linux_table_gives_listening_sockets_with_their_addresses_ports_and_inodes() {
        let sockets = parse_proc_net_tcp(PROC_TCP, false);

        // The established connection (state 01) is not a listener.
        assert_eq!(sockets.len(), 3);
        assert_eq!(
            sockets[0],
            ProcSocket {
                addr: "127.0.0.1".parse().unwrap(),
                port: 3306,
                inode: 12345
            }
        );
        assert_eq!(
            sockets[1],
            ProcSocket {
                addr: "0.0.0.0".parse().unwrap(),
                port: 80,
                inode: 22222
            }
        );
        assert_eq!(
            sockets[2],
            ProcSocket {
                addr: "192.168.0.5".parse().unwrap(),
                port: 3306,
                inode: 44444
            }
        );
    }

    #[test]
    fn the_linux_ipv6_table_is_read_the_same_way() {
        let sockets = parse_proc_net_tcp(PROC_TCP6, true);

        assert_eq!(sockets.len(), 2);
        assert_eq!(
            sockets[0],
            ProcSocket {
                addr: "::1".parse().unwrap(),
                port: 9000,
                inode: 55555
            }
        );
        assert_eq!(
            sockets[1],
            ProcSocket {
                addr: "::".parse().unwrap(),
                port: 3306,
                inode: 66666
            }
        );
    }

    #[test]
    fn a_damaged_linux_table_is_skipped_not_a_panic() {
        assert!(parse_proc_net_tcp("", false).is_empty());
        assert!(parse_proc_net_tcp("header only", false).is_empty());
        assert!(parse_proc_net_tcp("h\n 0: nonsense 00:0 0A 0:0 0:0 0 0 0 1\n", false).is_empty());
        assert!(parse_proc_net_tcp("h\n 0: 0100007F:0CEA x 0A\n", false).is_empty());
        assert!(parse_proc_net_tcp("h\n 0: ZZ:0CEA 0:0 0A 0:0 0:0 0 0 0 1\n", false).is_empty());
        assert!(parse_proc_net_tcp("h\n 0: 0100:0CEA 0:0 0A 0:0 0:0 0 0 0 1\n", true).is_empty());
    }

    #[test]
    fn lsof_output_gives_each_process_with_its_sockets() {
        let text = "p812\ncmysqld\nn*:3306\nn[::1]:3306\np901\ncControlCenter\nn127.0.0.1:7000\nn*:5000\np77\ncnode\nnnot-an-address\nn*:notaport\n";

        let sockets = parse_lsof(text);

        assert_eq!(sockets.len(), 4);
        assert_eq!(
            sockets[0],
            LsofSocket {
                pid: 812,
                command: "mysqld".into(),
                addr: "0.0.0.0".parse().unwrap(),
                port: 3306
            }
        );
        assert_eq!(sockets[1].addr, "::1".parse::<IpAddr>().unwrap());
        assert_eq!(sockets[2].command, "ControlCenter");
        assert_eq!((sockets[2].pid, sockets[2].port), (901, 7000));
        assert!(parse_lsof("").is_empty() && parse_lsof("garbage\n\nx\n").is_empty());
    }

    #[test]
    fn only_addresses_that_take_the_port_from_the_loopback_clash() {
        let clashes = |text: &str| clashes_with_loopback(text.parse().unwrap());

        assert!(clashes("0.0.0.0"));
        assert!(clashes("127.0.0.1"));
        assert!(clashes("::"));
        assert!(clashes("::ffff:127.0.0.1"));
        assert!(
            !clashes("::1"),
            "IPv6 loopback is another address from 127.0.0.1"
        );
        assert!(!clashes("192.168.0.5"), "a network card's own address");
        assert!(!clashes("10.0.0.1"));
    }
}
