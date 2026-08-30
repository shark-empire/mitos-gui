use std::env;
use std::os::unix::net::UnixDatagram;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::AsRawFd;

/// Sends the sd_notify READY=1 signal to the supervisor (mitos-init).
pub fn send_ready() {
    let Some(socket_path) = env::var_os("NOTIFY_SOCKET") else {
        return; // Not running under a supervisor
    };

    let path_bytes = socket_path.as_bytes();
    let is_abstract = path_bytes.first() == Some(&b'@');
    
    let sock = match UnixDatagram::unbound() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("MITOS GUI: notify socket creation failed: {e}");
            return;
        }
    };

    if is_abstract {
        // Abstract sockets require raw libc because std::os::unix::net 
        // doesn't support the null-byte prefix natively.
        let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        addr.sun_family = libc::AF_UNIX as _;
        
        let len = path_bytes.len().min(addr.sun_path.len());
        addr.sun_path[0] = 0; // Null byte for abstract namespace
        for i in 1..len {
            addr.sun_path[i] = path_bytes[i] as i8;
        }
        
        let addr_len = std::mem::size_of::<libc::sa_family_t>() + len;
        let msg = b"READY=1\n";
        
        let ret = unsafe {
            libc::sendto(
                sock.as_raw_fd(),
                msg.as_ptr() as *const std::os::raw::c_void,
                msg.len(),
                0,
                &addr as *const _ as *const libc::sockaddr,
                addr_len as libc::socklen_t,
            )
        };
        
        if ret < 0 {
            tracing::warn!("MITOS GUI: failed to send READY=1 (abstract socket)");
        } else {
            tracing::info!("MITOS GUI: Sent READY=1 to mitos-init");
        }
    } else {
        // Standard filesystem socket
        if let Err(e) = sock.send_to(b"READY=1\n", &socket_path) {
            tracing::warn!("MITOS GUI: failed to send READY=1 to {socket_path:?}: {e}");
        } else {
            tracing::info!("MITOS GUI: Sent READY=1 to mitos-init");
        }
    }
}
