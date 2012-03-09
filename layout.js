document._setMutationHandler(function(mut) {
    // 10: layout event
    postMessage(10, JSON.stringify(mut));
});
