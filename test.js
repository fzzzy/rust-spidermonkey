
print("Hello, world!");
print(document);
document._setMutationHandler(function(mut) {
    print(JSON.stringify(mut));
});

// The network layer sometimes deadlocks; disable for now
//window.location = "http://127.0.0.1/";

postMessage(0, [12,34,"Hello!"]);

setTimeout(function() {print("timeout1")}, 100);
setTimeout(function() {print("timeout2")}, 200);
setTimeout(function() {print("timeout3")}, 300);
setTimeout(function() {print("timeout4!!!")}, 400);

