
print("Hello, world!");
print(document);
document._setMutationHandler(function(mut) {
    print(JSON.stringify(mut));
});

// Hack. File urls are not correctly parsed right now,
// but this syntax just happens to work.
window.location = "file:foo.html";

postMessage(4, [12,34,"Hello!"]);

/*
disabled until uv settles down

window.location = "http://127.0.0.1/";

setTimeout(function() {print("timeout1")}, 100);
setTimeout(function() {print("timeout2")}, 200);
setTimeout(function() {print("timeout3")}, 300);
setTimeout(function() {print("timeout4!!!")}, 400);
*/