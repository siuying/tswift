struct Percent {
    let value: Int
    init?(_ raw: Int) {
        guard (0 ... 100).contains(raw) else { return nil }
        self.value = raw
    }
}

if let p = Percent(50) {
    print("valid \(p.value)")
}
if Percent(150) == nil {
    print("invalid is nil")
}

class Connection {
    let port: Int
    init?(port: Int) {
        if port <= 0 { return nil }
        self.port = port
    }
}

if let c = Connection(port: 8080) {
    print("connected on \(c.port)")
}
if Connection(port: 0) == nil {
    print("bad port is nil")
}

struct Force {
    let raw: Int
    init!(_ v: Int) {
        if v < 0 { return nil }
        self.raw = v
    }
}

let f = Force(42)
print(f.raw)
