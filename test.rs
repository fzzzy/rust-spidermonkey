
use spidermonkey;
import spidermonkey::js;

use std;
import std::{ io, json, map, os, treemap, uv };

import ctypes::size_t;
import comm::{ port, chan, recv, send, select2 };
import core::error;


type element = {
    mut tag: str,
    mut attr: option::t<treemap::treemap<str, str>>,
    mut parent: uint,
    mut child: @mut[uint]
};


enum node {
    doctype(str, str, str, uint),
    procinst(str, uint),
    text(str, uint),
    element(element),
    nonode,
}


type document = {
    mut nodes: [mut node],
};


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

    js::set_version(cx, 185u);
    js::set_options(cx,
        js::options::varobjfix | js::options::methodjit);

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


fn on_js_msg(myid : int, out : chan<out_msg>, m : js::jsrust_message, childid : int, doc : @document) -> int {
    // messages from javascript
    alt m.level{
        0u32 { } // CONNECT
        1u32 { } // SEND
        2u32 { } // RECV
        3u32 { } // CLOSE
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
        7u32 { } // cast
        8u32 { } // SETTIMEOUT
        9u32 { ret -1; } // exit
        10u32 { // layout event
            //std::io::println(m.message);
            alt json::from_str(m.message) {
                result::ok(v) {
                    on_layout_msg(doc, v);
                }
                _ { fail }
            }
        }
        _ { fail "unexpected case" }
    }
    ret childid;
}


fn on_ctl_msg(cx : js::context, global : js::object, msg : ctl_msg, checkwait : js::script, loadurl : js::script) {
    alt msg {
        load_url(x) {
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
                    log(error, #fmt("File not found: %s", script));
                    fail
                }
            }
        }
        io_cb(level, tag, timeout, _p, buf) {
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

fn on_layout_msg(doc: @document, msg_j : json::json) {
    let msg = alt msg_j {
            json::dict(x) { x }
            _ { fail }
        },
        typ = alt msg.get("type") {
            json::num(x) { x }
            _ { fail }
        },
        target = alt msg.get("target") {
            json::num(x) { x as uint }
            _ { fail }
        };

    alt typ {
        1. { // MUTATE VALUE
            let data = msg.get("data");
            std::io::println(#fmt("mutate %? %?", target, data));
        }
        2. { // MUTATE ATTR
            std::io::println(#fmt("mutate attr %?", target));
        }
        3. { // REMOVE ATTR
            std::io::println(#fmt("remove attr %?", target));
        }
        4. { // REMOVE
            let parent = alt doc.nodes[target - 1u] {
                doctype(x, y, z, p) { p }
                procinst(x, p) { p }
                text(x, p) { p }
                element(x) { x.parent }
                _ { fail }
            };
            doc.nodes[target - 1u] = nonode;
            alt doc.nodes[parent] {
                element(x) {
                    *x.child = vec::filter(copy *x.child, {|id|
                        log(error, ("asdf", id, target));
                        id != target
                    });
                    std::io::println(#fmt("remove %? %?", target, x.child));
                }
                _ { fail }
            }
            std::io::println(#fmt("nodes %? %?", target, doc.nodes));
        }
        5. { // MOVE
            std::io::println(#fmt("move %?", target));
        }
        6. { // INSERT
            let nid = alt msg.get("nid") {
                    json::num(x) { x as uint }
                    _ { fail }
                },
                index = alt msg.get("index") {
                    json::num(x) { x as uint }
                    _ { fail }
                },
                child = msg.get("child"),
                parent = doc.nodes[target - 1u];

            std::io::println(#fmt(
                "insert %? into %? at index %?: %s",
                nid, target, index, json::to_str(child)));

            let elt = alt child {
                json::string(s) { text(s, target) }
                json::dict(m) { 
                    alt m.find("html") {
                        option::some(json::string(tn)) {
                            element({
                                mut tag: tn,
                                mut attr: option::none,
                                mut parent: target,
                                mut child: @mut[]})
                        }
                        _ {
                            alt m.find("doctype") {
                                option::some(json::string(dt)) {
                                    doctype(dt, "", "", target)
                                }
                                _ {
                                    fail
                                }
                            }
                        }
                    }
                }
                _ { fail }
            };

            if nid - 1u == vec::len(doc.nodes) {
                doc.nodes += [mut elt];
            } else {
                doc.nodes[nid - 1u] = elt;
            }
            log(error, doc.nodes);

            alt parent {
                element(e) {
                    log(error, #fmt("slice %? %? %? %?", e, *e.child, index, vec::len(*e.child)));
                    *e.child = (
                        vec::slice(*e.child, 0u, index)
                        + [nid - 1u]
                        + vec::slice(*e.child, index, vec::len(*e.child)));
                    log(error, #fmt("%? %?", e, *e.child));
                }
                nonode { 
                    
                }
                _ { log(error, parent); fail }
            }

        }
        _y {
            log(error, _y);
        }
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

    let checkwait = js::compile_script(cx, global, str::bytes("if (XMLHttpRequest.requests_outstanding === 0) jsrust_exit();"), "io", 0u),
        loadurl = js::compile_script(cx, global, str::bytes("try { _resume(9, _data, 0) } catch (e) { print(e + '\\n' + e.stack) } _data = undefined;"), "io", 0u);

    if str::len(myurl) > 4u && (
        str::eq(str::slice(myurl, 0u, 4u), "http") ||
        str::eq(str::slice(myurl, 0u, 4u), "file")) {
        send(msg_chan, load_url(myurl));
    } else {
        let strlen = str::len(myurl);
        if str::eq(str::slice(myurl, strlen - 5u, strlen), ".html") {
            // hack: file urls aren't exactly the right format yet
            send(msg_chan, load_url(#fmt("file:%s", myurl)))
        } else {
            send(msg_chan, load_script(myurl));
        }
    }

    let exit = false,
        childid = 0,
        doc : @document = @{
            mut nodes: [
                mut element({
                    mut tag: "Document",
                    mut attr: option::none,
                    mut parent: 0u,
                    mut child: @mut[2u, 3u]}),
                doctype("", "", "", 0u),
                element({
                    mut tag: "html",
                    mut attr: option::none,
                    mut parent: 0u,
                    mut child: @mut[]})]};


    while !exit {
        alt select2(js_port, msg_port) {
            either::left(m) {
                childid = on_js_msg(myid, out, m, childid, doc);
                if childid == -1 {
                    send(out, exitproc);
                    exit = true;
                }
            }
            either::right(msg) {
                on_ctl_msg(cx, global, msg, checkwait, loadurl);
            }
        }
    }
}


fn main(args : [str]) {
    let maxbytes = 32u32 * 1024u32 * 1024u32,
        map = treemap::init();

    let stdoutport = port::<out_msg>(),
        stdoutchan = chan(stdoutport),
        sendchanport = port::<(int, chan<ctl_msg>)>(),
        sendchanchan = chan(sendchanport);

    let argc = vec::len(args),
        argv = if argc == 1u {
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
            stdout(x) { std::io::println(x); }
            stderr(x) { log(error, x); }
            spawn(id, src) {
                log(error, ("spawn", id, src));
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

