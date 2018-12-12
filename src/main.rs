extern crate dbus;

use std::env;
use std::error::Error;
use std::process::Command;
use std::sync::{Arc,Mutex};
use std::sync::atomic::{AtomicBool,Ordering};
use std::thread;
use std::time::{Duration,SystemTime};

use dbus::{BusType,Connection,Message,NameFlag};
use dbus::tree::Factory;

fn main() {
    let args : Vec<String> = env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "quit" => {
                call_it_quit();
                return
            }
            "disable" => {
                if args.len() == 2 {
                    call_disable(None);
                } else {
                    call_disable(Some(args[2].parse().unwrap()));
                };
                return
            }
            "enable" => {
                call_enable();
                return
            }
            "status" => {
                call_status();
                return
            }
            _ => {
                println!("Wrong argument: {:?}", args);
                return
            }
        }
    }

    let quitter = Arc::new(AtomicBool::new(false));
    let dbus_quitter = quitter.clone();
    let do_disable = Arc::new(AtomicBool::new(false));
    let dbus_do_disable = do_disable.clone();
    let enable_timestamp : Arc<Mutex<Option<SystemTime>>> = Arc::new(Mutex::new(None));
    let dbus_enable_timestamp = enable_timestamp.clone();

    thread::spawn(move || {
        let connection = initialize_connection(dbus_quitter, dbus_do_disable, dbus_enable_timestamp).unwrap();
        loop {
            connection.incoming(30_000).next();
        }
    });

    loop {
        thread::sleep(Duration::new(1,0));
        if quitter.load(Ordering::Relaxed) {
            break;
        }
        {
            let mut ts_locked = enable_timestamp.lock().unwrap();
            match *ts_locked {
                None => {}
                Some(timestamp) => {
                    if SystemTime::now() > timestamp {
                        println!("Screensaver enabled, timeout passed");
                        do_disable.store(false, Ordering::Relaxed);
                        *ts_locked = None;
                        continue;
                    }
                }
            }
        }
        if !do_disable.load(Ordering::Relaxed) {
            continue;
        }
        if screensaver_activated() {
            continue;
        }
        Command::new("sh")
            .arg("-c").arg("xscreensaver-command -deactivate")
            .output().expect("Failed to run 'xscreensaver-command -deactivate'");
    }
}

fn initialize_connection(quitter : Arc<AtomicBool>, do_disable : Arc<AtomicBool>, enable_timestamp : Arc<Mutex<Option<SystemTime>>>) -> Result<Connection, Box<dyn Error>> {
    let c = Connection::get_private(BusType::Session)?;
    c.register_name("net.andresovi.xees", NameFlag::ReplaceExisting as u32)?;
    let f = Factory::new_fn::<()>();
    let enable_do_disable = do_disable.clone();
    let status_do_disable = do_disable.clone();
    let enable_enable_timestamp = enable_timestamp.clone();
    let tree = f.tree(()).add(f.object_path("/", ()).introspectable().add(
            f.interface("net.andresovi.xees", ())
                        .add_m(
                            f.method("Enable", (), move |m| {
                                println!("Screensaver enabled");
                                enable_do_disable.store(false, Ordering::Relaxed);
                                *enable_enable_timestamp.lock().unwrap() = None;
                                Ok(vec![m.msg.method_return().append1("ok")])
                            }).outarg::<&str,_>("reply")
                            )
                        .add_m(
                            f.method("Disable", (), move |m| {
                                do_disable.store(true, Ordering::Relaxed);
                                match m.msg.read1() {
                                    Ok(val)  => {
                                        println!("Screensaveer disabled for {}s", val);
                                        *enable_timestamp.lock().unwrap() = Some(SystemTime::now() + Duration::new(val,0));
                                    }
                                    Err(_) => { // Missing or invalid duration => disable forever
                                        println!("Screensaver disabled indefinitely");
                                        *enable_timestamp.lock().unwrap() = None;
                                    }
                                };
                                Ok(vec![m.msg.method_return().append1("ok")])
                            })
                            .inarg::<u64,_>("duration")
                            .outarg::<&str,_>("reply")
                            )
                        .add_m(
                            f.method("Status", (), move |m| {
                                println!("Status called");
                                let msg = match status_do_disable.load(Ordering::Relaxed) {
                                    true => "Disabled",
                                    false => "Enabled"
                                };
                                Ok(vec![m.msg.method_return().append1(msg)])
                            })
                            )
                        .add_m(
                            f.method("Quit", (), move |m| {
                                quitter.store(true, Ordering::Relaxed);
                                Ok(vec![m.msg.method_return().append1("quitting")])
                            }).outarg::<&str,_>("reply")
                            )
        ));
    tree.set_registered(&c, true)?;
    c.add_handler(tree);
    Ok(c)
}

fn call_it_quit() {
    let connection = Connection::get_private(BusType::Session).unwrap();
    let m = Message::new_method_call("net.andresovi.xees", "/", "net.andresovi.xees", "Quit").unwrap();
    connection.send_with_reply_and_block(m, 2000).unwrap();
}

fn screensaver_activated() -> bool {
    /*
     * [fandres@greed ~]$ while true; do xscreensaver-command -time; sleep 5; done
     * XScreenSaver 5.40: screen non-blanked since Tue Dec  4 11:15:28 2018 (hack #154)
     * XScreenSaver 5.40: screen locked since Tue Dec  4 11:38:46 2018 (hack #212)
    */
    let output = Command::new("sh")
        .arg("-c").arg("xscreensaver-command -time")
        .output().expect("Failed to run 'xscreensaver -time'");
    if String::from_utf8_lossy(&output.stdout).contains("screen locked") { // at least I don't have two problems now
        return true
    }
    return false
}

fn call_disable(period : Option<u64>) {
    let connection = Connection::get_private(BusType::Session).unwrap();
    let mut m = Message::new_method_call("net.andresovi.xees", "/", "net.andresovi.xees", "Disable")
        .unwrap();
    if let Some(seconds) = period {
        m = m.append1(seconds);
    }
    connection.send_with_reply_and_block(m, 2000).unwrap();
}

fn call_enable() {
    let connection = Connection::get_private(BusType::Session).unwrap();
    let m = Message::new_method_call("net.andresovi.xees", "/", "net.andresovi.xees", "Enable").unwrap();
    connection.send_with_reply_and_block(m, 2000).unwrap();
}

fn call_status() {
    let connection = Connection::get_private(BusType::Session).unwrap();
    let m = Message::new_method_call("net.andresovi.xees", "/", "net.andresovi.xees", "Status").unwrap();
    let status : String = connection.send_with_reply_and_block(m, 2000).unwrap().get1().unwrap();
    println!("{}", status);
}
