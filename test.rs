use spidermonkey;
import spidermonkey::js;

import ctypes::size_t;
import comm::{ port, chan, recv, send, select2 };

use std;

import std::{ io, os, treemap, uv };

enum child_message {
    io_cb(u32, u32, u32, u32, str),
    stdout(str),
    stderr(str),
    spawn(str, str),
    cast(str, str),
    load_url(str),
    load_script(str),
    exitproc,
    done,
}


fn make_actor(myid : int, myurl : str, maxbytes : u32, out : chan<child_message>, sendchan : chan<(int, chan<child_message>)>) {
	/*let CONN = 0u32,
		SEND = 1u32,
		RECV = 2u32,
		CLOSE = 3u32,
		TIME = 8u32;*/

    let rt = js::get_thread_runtime(maxbytes);
    let msg_port = port::<child_message>();
    let msg_chan = chan(msg_port);
	send(sendchan, (myid, msg_chan));
    //let senduv_port = port::<chan<uv::uv_operation>>();

    let js_port = port::<js::jsrust_message>();
 
    let cx = js::new_context(rt, maxbytes as size_t);
    js::set_options(cx, js::options::varobjfix | js::options::methodjit);
    js::set_version(cx, 185u);

    let globclass = js::new_class({
		name: "global",
		flags: js::ext::get_global_class_flags() });
    let global = js::new_compartment_and_global_object(
        cx, globclass, js::null_principals());

    js::init_standard_classes(cx, global);
    js::ext::init_rust_library(cx, global);
    js::ext::set_msg_channel(cx, global, chan(js_port));

    alt std::io::read_whole_file("xmlhttprequest.js") {
        result::ok(file) {
            let script = js::compile_script(
                cx, global, file, "xmlhttprequest.js", 0u);
            js::execute_script(cx, global, script);
        }
        _ { fail }
    }
    alt std::io::read_whole_file("dom.js") {
        result::ok(file) {
            let script = js::compile_script(
                cx, global, file, "dom.js", 0u);
            js::execute_script(cx, global, script);
        }
        _ { fail }
    }
    if str::len(myurl) > 4u && str::eq(str::slice(myurl, 0u, 4u), "http") {
        send(msg_chan, load_url(myurl));
    } else {
        send(msg_chan, load_script(myurl));
    }

    let exit = false;
    let childid = 0;

    while !exit {
		alt select2(js_port, msg_port) {
			either::left(m) {
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
	                    childid = childid + 1;
	                }
	                7u32 { // cast
	                }
	                8u32 { // SETTIMEOUT
	                }
					9u32 { // exit
					}
	                _ {
	                    log(core::error, "...");
	                }
	            }
			}
			either::right(msg) {
		        alt msg {
		            load_url(x) {
		                //log(core::error, ("LOAD URL", x));
		                //js::begin_request(*cx);
		                js::set_data_property(cx, global, x);
		                let code = "try { _resume(5, _data, 0) } catch (e) { print(e + '\\n' + e.stack) } _data = undefined;";
		                let script = js::compile_script(cx, global, str::bytes(code), "io", 0u);
		                js::execute_script(cx, global, script);
		                //js::end_request(*cx);
		            }
		            load_script(script) {
		                alt std::io::read_whole_file(script) {
		                    result::ok(file) {
		                        let script = js::compile_script(
		                            cx, global,
									str::bytes(#fmt("try { %s } catch (e) { print(e + '\\n' + e.stack); }", str::from_bytes(file))),
									script, 0u);
		                        js::execute_script(cx, global, script);
		                        let checkwait = js::compile_script(
		                        cx, global, str::bytes("if (XMLHttpRequest.requests_outstanding === 0)  jsrust_exit();"), "io", 0u);
		                        js::execute_script(cx, global, checkwait);
		                    }
		                    _ {
		                        log(core::error, #fmt("File not found: %s", script));
		                        js::ext::rust_exit_now(0);
		                    }
		                }
		            }
		            io_cb(level, tag, timeout, _p, buf) {
		                log(core::error, ("io_cb", level, tag, timeout, buf));
		                js::begin_request(*cx);
		                js::set_data_property(cx, global, buf);
		                let code = #fmt("try { _resume(%u, _data, %u); } catch (e) { print(e + '\\n' + e.stack); }; _data = undefined;", level as uint, tag as uint);
		                let script = js::compile_script(cx, global, str::bytes(code), "io", 0u);
		                js::execute_script(cx, global, script);
		                js::end_request(*cx);
		            }
		            exitproc {
		            }
		            done {
		                exit = true;
		                send(out, done);
		            }
		            _ { fail "unexpected case" }
		        }
			}
		}
    }
}


fn main(args : [str]) {
    let maxbytes = 32u32 * 1024u32 * 1024u32;

    let stdoutport = port::<child_message>();
    let stdoutchan = chan(stdoutport);

    let sendchanport = port::<(int, chan<child_message>)>();
    let sendchanchan = chan(sendchanport);

    let map = treemap::init();

    let argc = vec::len(args);
    let argv = if argc == 1u {
        ["test.js"]
    } else {
        vec::slice(args, 1u, argc)
    };

    let left = 0;

    //let main_loop = uv::loop_new();
	//task::spawn {||
	//    uv::run(main_loop);
	//};

    for x in argv {
        left += 1;
        task::spawn {||
            make_actor(left, x, maxbytes, stdoutchan, sendchanchan);
        };
    }
    let actorid = left;

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
                    make_actor(actorid, src, maxbytes, stdoutchan, sendchanchan);
                };
            }
            cast(id, msg) {}
            exitproc {
                left = left - 1;
                if left == 0 {
                    let n = @mutable 0;
                    fn t(n: @mutable int, &&_k: int, &&v: chan<child_message>) {
                        send(v, exitproc);
                        *n += 1;
                    }
                    treemap::traverse(map, bind t(n, _, _));
                    left = *n;
                }
            }
            done {
                left = left - 1;
                if left == 0 {
                    break;
                }
            }
            _ { fail "unexpected case" }
        }
    }
}

