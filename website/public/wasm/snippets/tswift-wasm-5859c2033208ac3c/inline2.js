
        export function tswift_host_services() {
            const s = globalThis.tswiftHostServices;
            if (!Array.isArray(s)) return "";
            return s.filter((x) => typeof x === "string").join(",");
        }
    