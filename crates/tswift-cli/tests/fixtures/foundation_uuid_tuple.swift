import Foundation

// UUID.uuid exposes the raw 16 bytes as Darwin's `uuid_t`, a homogeneous
// (UInt8, ..., UInt8) tuple in network byte order.
let u = UUID(uuidString: "E621E1F8-C36C-495A-93FC-0C247A3E6E5F")!
let t = u.uuid
print(t.0, t.1, t.2, t.3, t.4, t.5, t.6, t.7)
print(t.8, t.9, t.10, t.11, t.12, t.13, t.14, t.15)

// Reassemble the byte tuple into an array and sum it (a stable, format-free
// cross-check that every element decoded correctly).
let bytes: [UInt8] = [
    t.0, t.1, t.2, t.3, t.4, t.5, t.6, t.7,
    t.8, t.9, t.10, t.11, t.12, t.13, t.14, t.15,
]
print(bytes.count)
print(bytes.map { Int($0) }.reduce(0, +))

// The nil UUID decodes to all-zero bytes.
let zero = UUID(uuidString: "00000000-0000-0000-0000-000000000000")!
print(zero.uuid.0, zero.uuid.15)
