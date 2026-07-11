
        export function tswift_http_call(requestJson) {
            const hook = globalThis.tswiftHttp;
            if (typeof hook !== "function") return null;
            try {
                return String(hook(requestJson));
            } catch (e) {
                return JSON.stringify({
                    error: "cannotConnectToHost",
                    message: String(e),
                });
            }
        }
    