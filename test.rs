
use spidermonkey;
import spidermonkey::js;

use std;
import std::{ io, os, treemap, uv };

import ctypes::size_t;
import comm::{ port, chan, recv, send, select2 };


enum out_msg {
    stdout(str),
    stderr(str),
    spawn(str, str),
    cast(str, str),
    exitproc,
}


enum ctl_msg {
    io_cb(u32, u32, u32, u32, str),
    load_url(str),
    load_script(str)
}


fn make_context(maxbytes : u32) -> (js::context, js::object) {
    let rt = js::get_thread_runtime(maxbytes),
        cx = js::new_context(rt, maxbytes as size_t);

    js::set_options(cx,
    js::options::varobjfix | js::options::methodjit);
    js::set_version(cx, 185u);

    let globclass = js::new_class({
    name: "global",
    flags: js::ext::get_global_class_flags() });

    let global = js::new_compartment_and_global_object(
        cx, globclass, js::null_principals());

    js::init_standard_classes(cx, global);
    js::ext::init_rust_library(cx, global);

    ret (cx, global);
}


fn run_script(cx : js::context, global : js::object, filename : str) {
    alt std::io::read_whole_file(filename) {
        result::ok(file) {
            let script = js::compile_script(
                cx, global, file, filename, 0u);
            js::execute_script(cx, global, script);
        }
        _ { fail #fmt("error reading file %s", filename) }
    }
}


fn on_js_msg(myid : int, out : chan<out_msg>, m : js::jsrust_message, childid : int) -> int {
    // messages from javascript
    alt m.level{
        0u32 { // CONNECT
        }
        1u32 { // SEND
        }
        2u32 { // RECV
        }
        3u32 { // CLOSE
        }
        4u32 { // stdout
            send(out, stdout(
                #fmt("[Actor %d] %s",
                myid, m.message)));
        }
        5u32 { // stderr
            send(out, stderr(
                #fmt("[ERROR %d] %s",
                myid, m.message)));
        }
        6u32 { // spawn
            send(out, spawn(
                #fmt("%d:%d", myid, childid),
                m.message));
            ret childid + 1;
        }
        7u32 { // cast
        }
        8u32 { // SETTIMEOUT
        }
        9u32 { // exit
            ret -1;
        }
        _ {
            log(core::error, "...");
            fail "unexpected case"
        }
    }
    ret childid;
}


fn on_ctl_msg(myid : int, cx : js::context, global : js::object, msg : ctl_msg, checkwait : js::script, loadurl : js::script) {
    alt msg {
        load_url(x) {
            log(core::error, x);
            js::set_data_property(cx, global, x);
            js::execute_script(cx, global, loadurl);
        }
        load_script(script) {
            alt std::io::read_whole_file(script) {
                result::ok(file) {
                    let script = js::compile_script(
                        cx, global,
                        str::bytes(
                            #fmt("try { %s } catch (e) { print(e + '\\n' + e.stack); }",
                            str::from_bytes(file))),
                            script, 0u);
                    js::execute_script(cx, global, script);
                    js::execute_script(cx, global, checkwait);
                }
                _ {
                    log(core::error, #fmt("File not found: %s", script));
                    js::ext::rust_exit_now(0);
                }
            }
        }
        io_cb(level, tag, timeout, _p, buf) {
            log(core::error, ("io_cb", myid, level, tag, timeout, buf));
            js::begin_request(*cx);
            js::set_data_property(cx, global, buf);
            let code = #fmt("try { _resume(%u, _data, %u); } catch (e) { print(e + '\\n' + e.stack); }; _data = undefined;", level as uint, tag as uint);
            let script = js::compile_script(cx, global, str::bytes(code), "io", 0u);
            js::execute_script(cx, global, script);
            js::end_request(*cx);
        }
        _ { fail "unexpected case" }
    }
}


fn run_actor(myid : int, myurl : str, maxbytes : u32, out : chan<out_msg>, sendchan : chan<(int, chan<ctl_msg>)>) {
    let msg_port = port::<ctl_msg>(),
    msg_chan = chan(msg_port);

    send(sendchan, (myid, msg_chan));

    let js_port = port::<js::jsrust_message>();

    let (cx, global) = make_context(maxbytes);
    js::ext::set_msg_channel(cx, global, chan(js_port));

    run_script(cx, global, "xmlhttprequest.js");
    run_script(cx, global, "dom.js");
    run_script(cx, global, "layout.js");

    let checkwait = js::compile_script(
        cx, global, str::bytes("if (XMLHttpRequest.requests_outstanding === 0) jsrust_exit();"), "io", 0u),
        loadurl = js::compile_script(cx, global, str::bytes("try { _resume(9, _data, 0) } catch (e) { print(e + '\\n' + e.stack) } _data = undefined;"), "io", 0u);

    if str::len(myurl) > 4u && (
        str::eq(str::slice(myurl, 0u, 4u), "http") ||
        str::eq(str::slice(myurl, 0u, 4u), "file")) {
            log(core::error, "loadurl");
        send(msg_chan, load_url(myurl));
    } else {
        send(msg_chan, load_script(myurl));
    }

    let exit = false,
    childid = 0;

    while !exit {
        alt select2(js_port, msg_port) {
            either::left(m) {
                childid = on_js_msg(myid, out, m, childid);
                if childid == -1 {
                    send(out, exitproc);
                    exit = true;
                }
            }
            either::right(msg) {
                on_ctl_msg(myid, cx, global, msg, checkwait, loadurl);
            }
        }
    }
}


fn main(args : [str]) {
    let maxbytes = 32u32 * 1024u32 * 1024u32;

    let stdoutport = port::<out_msg>();
    let stdoutchan = chan(stdoutport);

    let sendchanport = port::<(int, chan<ctl_msg>)>();
    let sendchanchan = chan(sendchanport);

    let map = treemap::init();

    let argc = vec::len(args);
    let argv = if argc == 1u {
        ["test.js"]
    } else {
        vec::slice(args, 1u, argc)
    };

    let left = 0,
    actorid = 0;

    for x in argv {
        left += 1;
        actorid += 1;
        task::spawn {||
            run_actor(actorid, x, maxbytes, stdoutchan, sendchanchan);
        };
    }

    for _x in argv {
        let (theid, thechan) = recv(sendchanport);
        treemap::insert(map, theid, thechan);
    }

    while true {
        alt recv(stdoutport) {
            stdout(x) { log(core::error, x); }
            stderr(x) { log(core::error, x); }
            spawn(id, src) {
                log(core::error, ("spawn", id, src));
                actorid = actorid + 1;
                left = left + 1;
                task::spawn {||
                    run_actor(actorid, src, maxbytes, stdoutchan, sendchanchan);
                };
                let (theid, thechan) = recv(sendchanport);
                treemap::insert(map, theid, thechan);
            }
            cast(id, msg) {}
            exitproc {
                left = left - 1;
                if left == 0 {
                    break;
                }
            }
            _ { fail "unexpected case" }
        }
    }
}

