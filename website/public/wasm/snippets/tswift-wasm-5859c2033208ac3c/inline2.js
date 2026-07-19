
        export function tswift_host_call(name, argsJson) {
            const hook = globalThis.tswiftHost;
            if (typeof hook !== "function") return null;
            try {
                const result = hook(name, argsJson);
                return result == null ? "null" : String(result);
            } catch (e) {
                return JSON.stringify({ "$hostError": String(e) });
            }
        }
    