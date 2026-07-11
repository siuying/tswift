import Foundation
import TSwiftFFI

/// Real-Foundation host backing for `tswift.defaults.*` / `tswift.fs.*` (see
/// `crates/tswift-foundation/src/user_defaults.rs` / `file_manager.rs` for the
/// wire contract this file implements, and `docs/adr/0014-host-services-web-ios.md`
/// for the cross-platform tier summary).
///
/// ## Platform tier: iOS (native — the same class as the CLI's backing, not a
/// degraded one)
///
/// Every call delegates straight to the real `UserDefaults`/`FileManager`
/// this process is handed: `UserDefaults.standard` (or a caller-supplied
/// suite) and `FileManager.default`, operating on whatever path the script
/// passes. There is no extra sandboxing layer here on top of Foundation's own
/// — the OS itself already confines the app to its container, exactly the
/// same trust boundary a hand-written Swift app gets from `FileManager`
/// directly. This mirrors the native CLI's backing
/// (`crates/tswift-cli/src/fs.rs`/`defaults.rs`): real storage, unrooted,
/// gated only by the platform's own access control.
///
/// ## Usage
///
/// ```swift
/// let context = TSwiftContext()
/// try context.installFoundationHostServices()
/// let result = TSwiftCore.run(script, in: context)
/// ```
extension TSwiftContext {
    /// Declare `tswift.defaults` and `tswift.fs`, and register their host
    /// functions backed by `defaults`/`fileManager`. Call once per context,
    /// before the first `run`/SwiftUI compile that needs `UserDefaults` or
    /// `FileManager`.
    @discardableResult
    public func installFoundationHostServices(
        defaults: UserDefaults = .standard,
        fileManager: FileManager = .default
    ) throws -> Self {
        try declareHostService("tswift.defaults")
        try declareHostService("tswift.fs")
        try installDefaultsHostFunctions(defaults: defaults)
        try installFileSystemHostFunctions(fileManager: fileManager)
        return self
    }

    // MARK: - tswift.defaults.*

    private func installDefaultsHostFunctions(defaults: UserDefaults) throws {
        try registerHostFunction(
            .init(
                name: "tswift.defaults.set",
                parameters: [.init(label: "key", type: .string), .init(label: "value", type: .string)]
            )
        ) { args in
            let key = args[0] as? String ?? ""
            let value = args[1] as? String ?? ""
            // `value` is already the JSON encoding of the stored Swift value
            // (see the module doc) \u2014 stored verbatim, exactly like the CLI's
            // `HashMap<String, String>` backing.
            defaults.set(value, forKey: key)
            return nil
        }

        try registerHostFunction(
            .init(
                name: "tswift.defaults.get",
                parameters: [.init(label: "key", type: .string)],
                returns: .optional(.string)
            )
        ) { args in
            let key = args[0] as? String ?? ""
            // Returning the raw stored JSON text as a Swift `String` is
            // correct here: `HostFunctionBox.encode` re-encodes it as a JSON
            // string, producing the required double-encoded reply (the
            // `String?` wire type's content IS the stored value's own JSON
            // encoding) \u2014 same trick the CLI backing uses.
            return defaults.string(forKey: key)
        }

        try registerHostFunction(
            .init(
                name: "tswift.defaults.remove",
                parameters: [.init(label: "key", type: .string)]
            )
        ) { args in
            let key = args[0] as? String ?? ""
            defaults.removeObject(forKey: key)
            return nil
        }
    }

    // MARK: - tswift.fs.*

    private func installFileSystemHostFunctions(fileManager: FileManager) throws {
        try registerHostFunction(
            .init(
                name: "tswift.fs.exists",
                parameters: [.init(label: "path", type: .string)],
                returns: .bool
            )
        ) { args in
            fileManager.fileExists(atPath: Self.path(args, 0))
        }

        try registerHostFunction(
            .init(
                name: "tswift.fs.isDirectory",
                parameters: [.init(label: "path", type: .string)],
                returns: .bool
            )
        ) { args in
            var isDir: ObjCBool = false
            let exists = fileManager.fileExists(atPath: Self.path(args, 0), isDirectory: &isDir)
            return exists && isDir.boolValue
        }

        try registerHostFunction(
            .init(
                name: "tswift.fs.read",
                parameters: [.init(label: "path", type: .string)],
                returns: .optional(.string)
            )
        ) { args in
            guard let data = fileManager.contents(atPath: Self.path(args, 0)) else { return nil }
            return data.base64EncodedString()
        }

        try registerHostFunction(
            .init(
                name: "tswift.fs.list",
                parameters: [.init(label: "path", type: .string)],
                returns: .array(.string),
                throwing: true
            )
        ) { args in
            let path = Self.path(args, 0)
            do {
                return try fileManager.contentsOfDirectory(atPath: path).sorted()
            } catch {
                throw TSwiftHostFunctionError(Self.describe("list", path, error))
            }
        }

        try registerHostFunction(
            .init(
                name: "tswift.fs.mkdir",
                parameters: [
                    .init(label: "path", type: .string),
                    .init(label: "withIntermediateDirectories", type: .bool),
                ],
                throwing: true
            )
        ) { args in
            let path = Self.path(args, 0)
            let intermediate = args[1] as? Bool ?? (args[1] as? NSNumber)?.boolValue ?? false
            do {
                try fileManager.createDirectory(
                    atPath: path, withIntermediateDirectories: intermediate
                )
            } catch {
                throw TSwiftHostFunctionError(Self.describe("create directory", path, error))
            }
            return nil
        }

        try registerHostFunction(
            .init(
                name: "tswift.fs.remove",
                parameters: [.init(label: "path", type: .string)],
                throwing: true
            )
        ) { args in
            let path = Self.path(args, 0)
            do {
                try fileManager.removeItem(atPath: path)
            } catch {
                throw TSwiftHostFunctionError(Self.describe("remove", path, error))
            }
            return nil
        }

        try registerHostFunction(
            .init(
                name: "tswift.fs.write",
                parameters: [
                    .init(label: "path", type: .string),
                    .init(label: "content", type: .string),
                    .init(label: "atomically", type: .bool),
                ],
                returns: .bool
            )
        ) { args in
            let path = Self.path(args, 0)
            let contentB64 = args[1] as? String ?? ""
            let atomically = args[2] as? Bool ?? (args[2] as? NSNumber)?.boolValue ?? false
            guard let bytes = Data(base64Encoded: contentB64) else { return false }
            if atomically {
                return (try? bytes.write(to: URL(fileURLWithPath: path), options: .atomic)) != nil
            }
            return fileManager.createFile(atPath: path, contents: bytes)
        }

        try registerHostFunction(
            .init(
                name: "tswift.fs.copy",
                parameters: [.init(label: "from", type: .string), .init(label: "to", type: .string)],
                throwing: true
            )
        ) { args in
            let from = Self.path(args, 0)
            let to = Self.path(args, 1)
            do {
                try fileManager.copyItem(atPath: from, toPath: to)
            } catch {
                throw TSwiftHostFunctionError(Self.describe("copy", from, to, error))
            }
            return nil
        }

        try registerHostFunction(
            .init(
                name: "tswift.fs.move",
                parameters: [.init(label: "from", type: .string), .init(label: "to", type: .string)],
                throwing: true
            )
        ) { args in
            let from = Self.path(args, 0)
            let to = Self.path(args, 1)
            do {
                try fileManager.moveItem(atPath: from, toPath: to)
            } catch {
                throw TSwiftHostFunctionError(Self.describe("move", from, to, error))
            }
            return nil
        }
    }

    // MARK: - Helpers

    private static func path(_ args: [Any], _ index: Int) -> String {
        args[index] as? String ?? ""
    }

    /// Build a message shaped like the CLI backing's thrown errors
    /// (`couldn\u2019t <verb> \u201c<path>\u201d: <reason>`) so a script's caught error
    /// text reads the same across platforms, per the host-services doc.
    private static func describe(_ verb: String, _ path: String, _ error: Error) -> String {
        "couldn\u{2019}t \(verb) \u{201c}\(path)\u{201d}: \(error.localizedDescription)"
    }

    private static func describe(_ verb: String, _ from: String, _ to: String, _ error: Error) -> String {
        "couldn\u{2019}t \(verb) \u{201c}\(from)\u{201d} to \u{201c}\(to)\u{201d}: \(error.localizedDescription)"
    }
}
