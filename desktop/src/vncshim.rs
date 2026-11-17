//! A password prompt in front of a desktop that asks for none, for macOS Screen Sharing.
//!
//! Screen Sharing cannot connect to a VNC server whose only sign-in method is "none"; it asks for a password and
//! then gives up. The desktops ask for none on purpose (hosts/exo-desktop: a socket only the account can open,
//! reached only through SSH). So on a Mac the viewer is pointed at this instead: a local port that speaks the first
//! lines of the VNC protocol to both sides, tells the viewer "password" and the desktop "none", accepts whatever
//! password arrives, and from then on only passes bytes along. It listens on 127.0.0.1 and reaches nothing the
//! forwarded port beside it does not already offer to this computer, so it guards nothing and needs to.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};

const VERSION: &[u8; 12] = b"RFB 003.008\n";

/// Starts the front for the forwarded desktop on 127.0.0.1:`target` and returns the port the viewer should use.
/// The listener lives as long as the app; each viewer connection gets its own pair of threads.
pub fn front(target: u16) -> std::io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    std::thread::spawn(move || {
        for viewer in listener.incoming().flatten() {
            std::thread::spawn(move || { if let Err(e) = serve(viewer, target) { eprintln!("vncshim: {e}") } });
        }
    });
    Ok(port)
}

fn bad(what: &str) -> std::io::Error { std::io::Error::new(std::io::ErrorKind::InvalidData, what.to_string()) }

fn serve(mut viewer: TcpStream, target: u16) -> std::io::Result<()> {
    let mut desk = TcpStream::connect(("127.0.0.1", target))?;
    let mut twelve = [0u8; 12];

    // the desktop's side: its version, ours back, then its list of sign-in methods, of which "none" (1) is taken
    desk.read_exact(&mut twelve)?;
    if &twelve[..4] != b"RFB " { return Err(bad("not a VNC server")) }
    desk.write_all(VERSION)?;
    let mut n = [0u8; 1];
    desk.read_exact(&mut n)?;
    let mut kinds = vec![0u8; n[0] as usize];
    desk.read_exact(&mut kinds)?;
    if !kinds.contains(&1) { return Err(bad("the desktop asks for a password of its own")) }
    desk.write_all(&[1])?;
    let mut result = [0u8; 4];
    desk.read_exact(&mut result)?;
    if result != [0, 0, 0, 0] { return Err(bad("the desktop refused")) }

    // the viewer's side: one method offered, "password" (2); the challenge is answered with anything. A 3.7 or 3.8
    // viewer gets a list and picks, and so does Apple's 3.889, which libvncserver gives the list like 3.7/3.8; any
    // other version is read as 3.3 (RFC 6143, section 7.1.1), so it is told the method as a U32 and picks nothing.
    // macOS Screen Sharing 15.7.9 answers 3.3.
    viewer.write_all(VERSION)?;
    viewer.read_exact(&mut twelve)?;
    if matches!(&twelve, b"RFB 003.007\n" | b"RFB 003.008\n" | b"RFB 003.889\n") {
        viewer.write_all(&[1, 2])?;
        viewer.read_exact(&mut n)?;
        if n[0] != 2 { return Err(bad("the viewer chose a method that was not offered")) }
    } else {
        viewer.write_all(&[0, 0, 0, 2])?;
    }
    viewer.write_all(&[0x5a; 16])?;
    let mut answer = [0u8; 16];
    viewer.read_exact(&mut answer)?;
    viewer.write_all(&[0, 0, 0, 0])?;

    // from here both speak to each other; one side closing closes the other
    let (mut v2, mut d2) = (viewer.try_clone()?, desk.try_clone()?);
    let up = std::thread::spawn(move || { let _ = std::io::copy(&mut v2, &mut d2); let _ = d2.shutdown(Shutdown::Both); });
    let _ = std::io::copy(&mut desk, &mut viewer);
    let _ = viewer.shutdown(Shutdown::Both);
    let _ = up.join();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in desktop that asks for no password, then a viewer that answers with `version` and is asked for one:
    /// the viewer gets through and what each side sends afterwards reaches the other. `list` is whether that version
    /// gets the 3.7/3.8/3.889 list of methods and picks, or the 3.3 U32 and picks nothing (RFC 6143, section 7.1.1).
    fn a_viewer_reaches_the_desktop(version: &[u8; 12], list: bool) {
        let desk = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let target = desk.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut s, _) = desk.accept().unwrap();
            let mut b = [0u8; 12];
            s.write_all(VERSION).unwrap(); s.read_exact(&mut b).unwrap();
            s.write_all(&[1, 1]).unwrap();
            let mut pick = [0u8; 1]; s.read_exact(&mut pick).unwrap(); assert_eq!(pick, [1]);
            s.write_all(&[0, 0, 0, 0]).unwrap();
            let mut init = [0u8; 1]; s.read_exact(&mut init).unwrap(); assert_eq!(init, [1]);   // the viewer's ClientInit
            s.write_all(b"server-init").unwrap();
        });
        let mut v = TcpStream::connect(("127.0.0.1", front(target).unwrap())).unwrap();
        v.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        let mut b = [0u8; 12];
        v.read_exact(&mut b).unwrap(); assert_eq!(&b, VERSION);
        v.write_all(version).unwrap();
        if list {
            let mut kinds = [0u8; 2]; v.read_exact(&mut kinds).unwrap(); assert_eq!(kinds, [1, 2]);
            v.write_all(&[2]).unwrap();
        } else {
            let mut kind = [0u8; 4]; v.read_exact(&mut kind).unwrap(); assert_eq!(kind, [0, 0, 0, 2]);
        }
        let mut challenge = [0u8; 16]; v.read_exact(&mut challenge).unwrap();
        v.write_all(&[7; 16]).unwrap();
        let mut ok = [9u8; 4]; v.read_exact(&mut ok).unwrap(); assert_eq!(ok, [0, 0, 0, 0]);
        v.write_all(&[1]).unwrap();
        let mut init = [0u8; 11]; v.read_exact(&mut init).unwrap(); assert_eq!(&init, b"server-init");
    }

    /// Apple's 3.889 is the known exception to "any other version is 3.3": libvncserver gives it the list like 3.7/3.8.
    #[test]
    fn a_viewer_asked_for_a_password_reaches_a_desktop_that_asks_for_none() {
        a_viewer_reaches_the_desktop(b"RFB 003.008\n", true);
        a_viewer_reaches_the_desktop(b"RFB 003.007\n", true);
        a_viewer_reaches_the_desktop(b"RFB 003.889\n", true);
    }

    /// macOS Screen Sharing 15.7.9 answers with version 3.3. Sent the 3.8 list instead, it waited forever (issue #6).
    #[test]
    fn a_viewer_that_speaks_version_3_3_or_another_version_reaches_the_desktop_too() {
        a_viewer_reaches_the_desktop(b"RFB 003.003\n", false);
        a_viewer_reaches_the_desktop(b"RFB 003.005\n", false);
    }
}
