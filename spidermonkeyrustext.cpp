#include <js/jsapi.h>
#include <cassert>
#include <cstdlib>
#include <cstring>
#include <stdint.h>
#include <pthread.h>
#include <errno.h>

/*
 * Rust API declarations.
 *
 * TODO: Rust should expose a nice header file for this kind of thing.
 */

typedef uintptr_t rust_port_id;
typedef uintptr_t rust_task_id;
struct rust_port;
struct type_desc;

struct rust_chan_pkg {
    rust_task_id task;
    rust_port_id port;

    rust_chan_pkg() : task(0), port(0) {}
};

struct rust_s_shared_malloc_args {
    uintptr_t retval;
    size_t nbytes;
    type_desc *td;
};

extern "C" rust_port *new_port(size_t unit_sz);
extern "C" void del_port(rust_port *port);
extern "C" uintptr_t chan_id_send(const type_desc *t,
                                  rust_task_id target_task_id,
                                  rust_port_id target_port_id, void *sptr);
extern "C" void upcall_s_shared_malloc(rust_s_shared_malloc_args *args);

class rust_str {
private:
    uintptr_t size;
    uintptr_t pad;
    char data[0];

    rust_str() { /* Don't call me. */ }

public:
    static rust_str *make(const char *c_str) {
        uintptr_t len = strlen(c_str);
        size_t obj_len = sizeof(rust_str) + len + 1;
        rust_s_shared_malloc_args args = { 0, obj_len, NULL };
        upcall_s_shared_malloc(&args);

        rust_str *str = reinterpret_cast<rust_str *>(args.retval);
        str->size = len;
        strcpy(str->data, c_str);
        return str;
    }
};

/*
 * SpiderMonkey helpers, needed since Rust doesn't support C++ global
 * variables.
 */

extern "C" JSPropertyOp JSRust_GetPropertyStub() {
    return JS_PropertyStub;
}

extern "C" JSStrictPropertyOp JSRust_GetStrictPropertyStub() {
    return JS_StrictPropertyStub;
}

extern "C" JSEnumerateOp JSRust_GetEnumerateStub() {
    return JS_EnumerateStub;
}

extern "C" JSResolveOp JSRust_GetResolveStub() {
    return JS_ResolveStub;
}

extern "C" JSConvertOp JSRust_GetConvertStub() {
    return JS_ConvertStub;
}

extern "C" JSFinalizeOp JSRust_GetFinalizeStub() {
    return JS_FinalizeStub;
}

/* Port and channel constructors */

namespace {

struct jsrust_context_priv {
    const type_desc *msg_tydesc;
    rust_chan_pkg msg_chan;

    jsrust_context_priv() : msg_tydesc(NULL), msg_chan() {}
};

struct jsrust_message {
    uint32_t level;
    rust_str *message;
    uint32_t tag;
    uint32_t timeout;
    uint32_t pad;
};


void port_finalize(JSContext *cx, JSObject *obj) {
    rust_port *port = reinterpret_cast<rust_port *>(JS_GetPrivate(cx, obj));
    if (port)
        del_port(port);
}

JSClass port_class = {
    "Port",                         /* name */
    JSCLASS_HAS_PRIVATE,            /* flags */
    JS_PropertyStub,                /* addProperty */
    JS_PropertyStub,                /* delProperty */
    JS_PropertyStub,                /* getProperty */
    JS_StrictPropertyStub,          /* setProperty */
    JS_EnumerateStub,               /* enumerate */
    JS_ResolveStub,                 /* resolve */
    JS_ConvertStub,                 /* convert */
    port_finalize,                  /* finalize */
    JSCLASS_NO_OPTIONAL_MEMBERS
};

JSBool jsrust_new_port(JSContext *cx, uintN argc, jsval *vp) {
    jsval constructor = JS_THIS(cx, vp);
    JSObject *obj = JS_NewObject(
        cx, &port_class, NULL, JSVAL_TO_OBJECT(constructor));

    if (!obj) {
        JS_ReportError(cx, "Could not create Port");
        return JS_FALSE;
    }

    rust_port *port = new_port(sizeof(void *) * 2);
    JS_SetPrivate(cx, obj, port);
    JS_SET_RVAL(cx, vp, OBJECT_TO_JSVAL(obj));
    return JS_TRUE;
}

JSBool jsrust_port_channel(JSContext *cx, uintN argc, jsval *vp) {
    jsval self = JS_THIS(cx, vp);
    //rust_port *port = (rust_port *)JS_GetPrivate(cx, JSVAL_TO_OBJECT(cx, self));
    // todo make channel and return it
    JS_SET_RVAL(cx, vp, JSVAL_NULL);
    return JS_TRUE;
}

static JSFunctionSpec port_functions[] = {
    JS_FN("channel", jsrust_port_channel, 0, 0),
    JS_FS_END
};

static uint32_t io_op_num = 1;

enum IO_OP {
    CONNECT,
    SEND,
    RECV,
    CLOSE,
    STDOUT,
    STDERR,
    SPAWN,
    CAST,
    TIME,
    EXIT
};

uint32_t jsrust_send_msg(JSContext *cx, enum IO_OP op, rust_str *data, uint32_t req_id, uint32_t timeout) {
    void *priv_p = JS_GetContextPrivate(cx);
    assert(priv_p && "No private data associated with context!");
    jsrust_context_priv *priv =
        reinterpret_cast<jsrust_context_priv *>(priv_p);

    uint32_t my_num = req_id;
    if (!my_num) {
        my_num = io_op_num++;
    }

    jsrust_message evt = { op, data, my_num, timeout, 0 };

    chan_id_send(priv->msg_tydesc, priv->msg_chan.task,
                 priv->msg_chan.port, &evt);

    return my_num;
}

void jsrust_report_error(JSContext *cx, const char *c_message,
                         JSErrorReport *c_report)
{
    void *priv_p = JS_GetContextPrivate(cx);
    assert(priv_p && "No private data associated with context!");
    jsrust_context_priv *priv =
        reinterpret_cast<jsrust_context_priv *>(priv_p);

    rust_str *message = rust_str::make(c_message);

    jsrust_send_msg(cx, STDERR, message, 0, 0);
}

}   /* end anonymous namespace */

extern "C" JSContext *JSRust_NewContext(JSRuntime *rt, size_t size) {
    JSContext *cx = JS_NewContext(rt, size);
    if (!cx)
        return NULL;

    jsrust_context_priv *priv = new jsrust_context_priv();
    JS_SetContextPrivate(cx, priv);
    return cx;
}

// stolen from js shell
static JSBool JSRust_Print(JSContext *cx, uintN argc, jsval *vp) {
    jsval *argv;
    uintN i;
    JSString *str;
    char *bytes;

    printf("%p ", (void *)pthread_self());

    argv = JS_ARGV(cx, vp);
    for (i = 0; i < argc; i++) {
        str = JS_ValueToString(cx, argv[i]);
        if (!str)
            return JS_FALSE;
        bytes = JS_EncodeString(cx, str);
        if (!bytes)
            return JS_FALSE;
        printf("%s%s", i ? " " : "", bytes);
        JS_free(cx, bytes);
    }
    printf("\n");
    JS_SET_RVAL(cx, vp, JSVAL_VOID);
    return JS_TRUE;
}

static JSString *
FileAsString(JSContext *cx, const char *pathname)
{
    FILE *file;
    JSString *str = NULL;
    size_t len, cc;
    char *buf;

    file = fopen(pathname, "rb");
    if (!file) {
        JS_ReportError(cx, "can't open %s: %s", pathname, strerror(errno));
        return NULL;
    }

    if (fseek(file, 0, SEEK_END) != 0) {
        JS_ReportError(cx, "can't seek end of %s", pathname);
    } else {
        len = ftell(file);
        if (fseek(file, 0, SEEK_SET) != 0) {
            JS_ReportError(cx, "can't seek start of %s", pathname);
        } else {
            buf = (char*) JS_malloc(cx, len + 1);
            if (buf) {
                cc = fread(buf, 1, len, file);
                if (cc != len) {
                    JS_ReportError(cx, "can't read %s: %s", pathname,
                                   (ptrdiff_t(cc) < 0) ? strerror(errno) : "short read");
                } else {
                    jschar *ucbuf;
                    size_t uclen;

                    len = (size_t)cc;

                    if (!JS_DecodeUTF8(cx, buf, len, NULL, &uclen)) {
                        JS_ReportError(cx, "Invalid UTF-8 in file '%s'", pathname);
                        return NULL;
                    }

                    ucbuf = (jschar*)malloc(uclen * sizeof(jschar));
                    JS_DecodeUTF8(cx, buf, len, ucbuf, &uclen);
                    str = JS_NewUCStringCopyN(cx, ucbuf, uclen);
                    free(ucbuf);
                }
                JS_free(cx, buf);
            }
        }
    }
    fclose(file);

    return str;
}

static JSBool
JSRust_Read(JSContext *cx, uintN argc, jsval *vp)
{
    JSString *str;

    if (!argc)
        return JS_FALSE;

    str = JS_ValueToString(cx, JS_ARGV(cx, vp)[0]);
    if (!str)
        return JS_FALSE;
    JSAutoByteString filename(cx, str);
    if (!filename)
        return JS_FALSE;

    const char *pathname = filename.ptr();

    if (!(str = FileAsString(cx, pathname)))
        return JS_FALSE;
    *vp = STRING_TO_JSVAL(str);
    return JS_TRUE;
}

static JSFunctionSpec global_functions[] = {
    JS_FN("print", JSRust_Print, 0, 0),
    JS_FN("jsrust_read", JSRust_Read, 0, 0),
    JS_FS_END
};

extern "C" JSBool JSRust_InitRustLibrary(JSContext *cx, JSObject *global) {
    JSObject *result = JS_InitClass(
        cx, global, NULL,
        &port_class,
        jsrust_new_port,
        0, // 0 args
        NULL, // no properties
        port_functions,
        NULL, NULL);

    JS_DefineFunctions(cx, global, global_functions);

    return !!result;
}

JSBool JSRust_PostMessage(JSContext *cx, uintN argc, jsval *vp) {
    void *priv_p = JS_GetContextPrivate(cx);
    assert(priv_p && "No private data associated with context!");
    jsrust_context_priv *priv =
        reinterpret_cast<jsrust_context_priv *>(priv_p);

    uint32_t what = 0;
    JSString *thestr;
    JS_ConvertArguments(cx,
        2, JS_ARGV(cx, vp), "uS", &what, &thestr);

    const char *code = JS_EncodeString(cx, thestr);
    rust_str *message = rust_str::make(code);

    jsrust_send_msg(cx, (enum IO_OP)what, message, 0, 0);

    JS_SET_RVAL(cx, vp, JSVAL_NULL);
    return JS_TRUE;
}

static JSFunctionSpec postMessage_functions[] = {
    JS_FN("postMessage", JSRust_PostMessage, 0, 0),
    JS_FS_END
};


JSBool JSRust_Connect(JSContext *cx, uintN argc, jsval *vp) {
    JSString *a2str;

    JS_ConvertArguments(cx,
        1, JS_ARGV(cx, vp), "S", &a2str);

    rust_str *a2 = rust_str::make(
        JS_EncodeString(cx, a2str));

    uint32_t my_num = jsrust_send_msg(cx, CONNECT, a2, 0, 0);

    JS_SET_RVAL(cx, vp, INT_TO_JSVAL(my_num));
    return JS_TRUE;
}

JSBool JSRust_Send(JSContext *cx, uintN argc, jsval *vp) {
    uint32_t req_id;
    JSString *data;

    JS_ConvertArguments(cx,
        2, JS_ARGV(cx, vp), "uS", &req_id, &data);

    rust_str *data_rust = rust_str::make(
        JS_EncodeString(cx, data));

    jsrust_send_msg(cx, SEND, data_rust, req_id, 0);

    JS_SET_RVAL(cx, vp, JSVAL_NULL);
    return JS_TRUE;
}

JSBool JSRust_Recv(JSContext *cx, uintN argc, jsval *vp) {
    uint32_t req_id;
    JSString *amount_str;

    JS_ConvertArguments(cx,
        2, JS_ARGV(cx, vp), "uS", &req_id, &amount_str);

    rust_str *amount_rust = rust_str::make(
        JS_EncodeString(cx, amount_str));


    jsrust_send_msg(cx, RECV, amount_rust, req_id, 0);

    JS_SET_RVAL(cx, vp, JSVAL_NULL);
    return JS_TRUE;
}

JSBool JSRust_Timeout(JSContext *cx, uintN argc, jsval *vp) {
    uint32_t timeout;

    JS_ConvertArguments(cx,
        1, JS_ARGV(cx, vp), "u", &timeout);

    rust_str *nothing = rust_str::make("");

    int32_t my_num = jsrust_send_msg(cx, TIME, nothing, 0, timeout);

    JS_SET_RVAL(cx, vp, INT_TO_JSVAL(my_num));
    return JS_TRUE;
}

JSBool JSRust_Close(JSContext *cx, uintN argc, jsval *vp) {
    uint32_t req_id;

    JS_ConvertArguments(cx,
        1, JS_ARGV(cx, vp), "u", &req_id);

    rust_str *nothing = rust_str::make("");

    jsrust_send_msg(cx, CLOSE, nothing, req_id, 0);

    JS_SET_RVAL(cx, vp, JSVAL_NULL);
    return JS_TRUE;
}

JSBool JSRust_Exit(JSContext *cx, uintN argc, jsval *vp) {
    uint32_t req_id;

    rust_str *nothing = rust_str::make("");

    jsrust_send_msg(cx, EXIT, nothing, 0, 0);

    JS_SET_RVAL(cx, vp, JSVAL_NULL);
    return JS_TRUE;
}

static JSFunctionSpec io_functions[] = {
    JS_FN("jsrust_connect", JSRust_Connect, 1, 0),
    JS_FN("jsrust_send", JSRust_Send, 2, 0),
    JS_FN("jsrust_recv", JSRust_Recv, 2, 0),
    JS_FN("jsrust_close", JSRust_Close, 1, 0),
    JS_FN("jsrust_timeout", JSRust_Timeout, 2, 0),
    JS_FN("jsrust_exit", JSRust_Exit, 0, 0),
    JS_FS_END
};

extern "C" JSBool JSRust_SetMessageChannel(JSContext *cx,
                                         JSObject *global,
                                         const rust_chan_pkg *channel,
                                         const type_desc *tydesc) {
    void *priv_p = JS_GetContextPrivate(cx);
    assert(priv_p && "No private data associated with context!");
    jsrust_context_priv *priv =
        reinterpret_cast<jsrust_context_priv *>(priv_p);

    priv->msg_tydesc = tydesc;
    priv->msg_chan = *channel;

    JS_DefineFunctions(cx, global, postMessage_functions);
    JS_DefineFunctions(cx, global, io_functions);
    JS_SetErrorReporter(cx, jsrust_report_error);

    return JS_TRUE;
}

extern "C" JSBool JSRust_Exit(int code) {
    exit(code);
}

extern "C" void JSRust_SetDataOnObject(JSContext *cx, JSObject *obj, const char *val, uint32_t vallen) {
    JSString *valstr = JS_NewStringCopyN(cx, val, vallen);
    jsval *jv = (jsval *)malloc(sizeof(jsval));
    *jv = STRING_TO_JSVAL(valstr);
    JS_SetProperty(cx, obj, "_data", jv);
}

static pthread_mutex_t get_runtime_mutex = PTHREAD_MUTEX_INITIALIZER;
static pthread_key_t thread_runtime_key;
static int initialized = 0;

JSRuntime *jsrust_getthreadruntime(uint32_t max_bytes) {
    pthread_mutex_lock(&get_runtime_mutex);
    if (!initialized) {
        pthread_key_create(&thread_runtime_key, NULL);
        initialized = 1;
    }
    pthread_mutex_unlock(&get_runtime_mutex);

    JSRuntime *rt = (JSRuntime *)pthread_getspecific(thread_runtime_key);
    if (rt == NULL) {
        rt = JS_NewRuntime(max_bytes);
        pthread_setspecific(thread_runtime_key, (const void *)rt);
    }
    return rt;
}

extern "C" JSRuntime *JSRust_GetThreadRuntime(uint32_t max_bytes) {
    return jsrust_getthreadruntime(max_bytes);
}

extern "C" uint32_t JSRust_GetGlobalClassFlags() {
    return JSCLASS_GLOBAL_FLAGS;
}
