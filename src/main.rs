extern crate chrono;
extern crate dbus;
extern crate timer;

use std::env;
use std::error::Error;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool,Ordering};
use std::sync::mpsc::{channel,RecvTimeoutError,Sender};
use std::time::Duration;
use std::thread;

use dbus::{BusType,Connection,Message,NameFlag};
use dbus::tree::Factory;

use timer::{Guard,Timer};

/* TODO: do not run a deactivate thread if we are supposed to be enabled */

fn main() {
    let args : Vec<String> = env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "list" => {
                list_all();
                return
            }
            "quit" => {
                call_it_quit();
                return
            }
            "disable" => {
                let how_long : i64 = match args.len() {
                    2 => {
                        3600
                    }
                    _ => {
                        args[2].parse().unwrap()
                    }
                };
                call_disable(how_long);
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
    let disabler_quitter = quitter.clone();
    let do_disable = Arc::new(AtomicBool::new(false));
    let disabler_do_disable = do_disable.clone();

    let (channel_sender,channel_receiver) = channel();

    thread::spawn(move || { disabling_loop(disabler_quitter, disabler_do_disable) });
    let connection = initialize_connection(quitter.clone(), do_disable.clone(), channel_sender).unwrap();

    let timer = Timer::new();
    let mut _guard : Option<Guard> = None;

    loop {
        connection.incoming(1000).next();
        if quitter.load(Ordering::Relaxed) {
            break;
        }
        let received = channel_receiver.recv_timeout(Duration::new(1, 0));
        match received {
            Ok(val) => {
                match val {
                    None => { _guard = None }
                    Some(seconds) => {
                        let _timer_do_disable = do_disable.clone();
                        _guard = Some(timer.schedule_with_delay(chrono::Duration::seconds(seconds),
                                                                move || {_timer_do_disable.store(false, Ordering::Relaxed)}))
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => { continue }
            Err(err) => {
                println!("Error while receiving from the channel: {:?}", err);
                quitter.store(false, Ordering::Relaxed)
            }
        }
    }
}

fn initialize_connection(quitter : Arc<AtomicBool>, do_disable : Arc<AtomicBool>, channel_to_timer : Sender<Option<i64>>) -> Result<Connection, Box<dyn Error>> {
    let c = Connection::get_private(BusType::Session)?;
    c.register_name("net.andresovi.xees", NameFlag::ReplaceExisting as u32)?;
    let f = Factory::new_fn::<()>();
    let enable_do_disable = do_disable.clone();
    let status_do_disable = do_disable.clone();
    let enable_channel_to_timer = channel_to_timer.clone();
    let tree = f.tree(()).add(f.object_path("/", ()).introspectable().add(
            f.interface("net.andresovi.xees", ())
                        .add_m(
                            f.method("Enable", (), move |m| {
                                println!("=== Enable");
                                enable_do_disable.store(false, Ordering::Relaxed);
                                enable_channel_to_timer.send(None).expect("Cannot send a message to timer thread");
                                Ok(vec![m.msg.method_return().append1("ok")])
                            }).outarg::<&str,_>("reply")
                            )
                        .add_m(
                            f.method("Disable", (), move |m| {
                                let timeout = match m.msg.get1() {
                                    None => { Some(3600) }
                                    val  => { val } // TODO: ensure we get a i64 value here
                                };
                                println!("=== Disable {:?}", timeout);
                                do_disable.store(true, Ordering::Relaxed);
                                channel_to_timer.send(timeout).expect("Cannot send a message to timer thread");
                                Ok(vec![m.msg.method_return().append1("ok")])
                            })
                            .inarg::<&str,_>("duration")
                            .outarg::<&str,_>("reply")
                            )
                        .add_m(
                            f.method("Status", (), move |m| {
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
                        .add_m(
                            f.method("Test", (), |m| {
                                println!("{:?}", m.msg.get_items()); // print message items we got as arguments of a method call
                                Ok(vec![m.msg.method_return().append1("test_reply")])
                            })
                            .inarg::<&str,_>("duration")
                            .outarg::<&str,_>("reply")
                            )
        ));
    tree.set_registered(&c, true)?;
    c.add_handler(tree);
    Ok(c)
}

fn list_all() {
    let connection = Connection::get_private(BusType::Session).unwrap();
    let m = Message::new_method_call("net.andresovi.xees", "/", "net.andresovi.xees", "Test").unwrap().append("arg1");
    let r = connection.send_with_reply_and_block(m, 2000).unwrap();
    println!("---1\n{:?}", r.get_items()); // print message items we got in return
}

fn call_it_quit() {
    let connection = Connection::get_private(BusType::Session).unwrap();
    let m = Message::new_method_call("net.andresovi.xees", "/", "net.andresovi.xees", "Quit").unwrap();
    connection.send_with_reply_and_block(m, 2000).unwrap();
}

fn disabling_loop(quitter : Arc<AtomicBool>, do_disable : Arc<AtomicBool>) {
    loop {
        if quitter.load(Ordering::Relaxed) {
            break;
        }
        thread::sleep(Duration::new(30,0));
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

fn screensaver_activated() -> bool {
    /*
     * [fandres@greed ~]$ while true; do xscreensaver-command -time; sleep 5; done
     * XScreenSaver 5.40: screen non-blanked since Tue Dec  4 11:15:28 2018 (hack #154)
     * XScreenSaver 5.40: screen locked since Tue Dec  4 11:38:46 2018 (hack #212)
    */
    let output = Command::new("sh")
        .arg("-c").arg("xscreensaver-command -time")
        .output().expect("Failed to run 'xscreensaver -time'");
    if String::from_utf8_lossy(&output.stdout).contains("screen locked") { // at least now I don't have two problems
        return true
    }
    return false
}

fn call_disable(period : i64) {
    let connection = Connection::get_private(BusType::Session).unwrap();
    let m = Message::new_method_call("net.andresovi.xees", "/", "net.andresovi.xees", "Disable")
        .unwrap()
        .append1(period);
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
