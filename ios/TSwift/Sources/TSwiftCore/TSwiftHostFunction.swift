import Foundation
import TSwiftFFI

/// The host side of the runtime's host-native function bridge (Epic #246).
///
/// A host registers a named function with a compact signature and a Swift
/// closure; interpreted Swift can then call it like an ordinary free function.
/// The seam is synchronous (the interpreter is a cooperative single-threaded
/// executor), so the closure runs on the interpreting thread and must return a
/// value synchronously:
///
/// ```swift
/// let context = TSwiftContext()
/// context.registerHostFunction(
///     .init(name: "hostDeviceName", returns: .string)
/// ) { _ in UIDevice.current.name }
///
/// context.registerHostFunction(
///     .init(name: "hostHaptic", parameters: [.init(label: "style", type: .string)])
/// ) { args in
///     playHaptic(style: args[0] as? String ?? "light")
///     return nil            // a Void return
/// }
/// ```
///
/// Registered functions are owned by the `TSwiftContext`: their handler boxes
/// are retained for the context's lifetime and released on `deinit`, on
/// removal, or when replaced by name — nothing leaks across runs. They are
/// available in both one-shot runs and SwiftUI preview sessions on the context.

// MARK: - Signature description

/// A stage-1 host-function type: the shapes that map cleanly onto the runtime's
/// JSON wire (scalars, optionals, arrays, string-keyed dictionaries).
public enum TSwiftHostType: Sendable {
    case void
    case bool
    case int
    case double
    case string
    /// `T?` — a present value or `nil`.
    indirect case optional(TSwiftHostType)
    /// `[T]`.
    indirect case array(TSwiftHostType)
    /// `[String: V]` — string-keyed dictionary.
    indirect case dictionary(TSwiftHostType)

    /// The compact JSON encoding: scalars are bare strings, compound types are
    /// single-key objects (`{"optional": T}`, `{"array": T}`, `{"dictionary": V}`).
    var jsonValue: Any {
        switch self {
        case .void: return "Void"
        case .bool: return "Bool"
        case .int: return "Int"
        case .double: return "Double"
        case .string: return "String"
        case let .optional(inner): return ["optional": inner.jsonValue]
        case let .array(inner): return ["array": inner.jsonValue]
        case let .dictionary(value): return ["dictionary": value.jsonValue]
        }
    }
}

/// One parameter of a host-function signature: an optional external label and a
/// type. A `nil` label means the argument is unlabelled (Swift's `_`).
public struct TSwiftHostParameter: Sendable {
    public let label: String?
    public let type: TSwiftHostType

    public init(label: String? = nil, type: TSwiftHostType) {
        self.label = label
        self.type = type
    }

    var jsonObject: [String: Any] {
        var object: [String: Any] = ["type": type.jsonValue]
        if let label { object["label"] = label }
        return object
    }
}

/// A host-function signature: the name interpreted code calls, its parameters,
/// its return type, and whether it may throw a catchable Swift error.
public struct TSwiftHostSignature: Sendable {
    public let name: String
    public let parameters: [TSwiftHostParameter]
    public let returns: TSwiftHostType
    public let throwing: Bool

    public init(
        name: String,
        parameters: [TSwiftHostParameter] = [],
        returns: TSwiftHostType = .void,
        throwing: Bool = false
    ) {
        self.name = name
        self.parameters = parameters
        self.returns = returns
        self.throwing = throwing
    }

    /// Encode to the FFI signature JSON (see `crates/tswift-core/src/host_bridge.rs`).
    func toJSON() -> String {
        let object: [String: Any] = [
            "name": name,
            "params": parameters.map(\.jsonObject),
            "returns": returns.jsonValue,
            "throws": throwing,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: object),
              let json = String(data: data, encoding: .utf8)
        else { return "{\"name\":\"\(name)\"}" }
        return json
    }
}

// MARK: - Errors

/// A failure raised inside a host-function closure. The `message` surfaces to
/// interpreted Swift as a catchable error naming the function (requires the
/// signature to be declared `throwing: true`).
public struct TSwiftHostFunctionError: Error, Sendable {
    public let message: String
    public init(_ message: String) { self.message = message }
}

/// A failure registering a host function (e.g. a malformed signature).
public struct TSwiftHostRegistrationError: Error, Sendable {
    public let message: String
    public init(_ message: String) { self.message = message }
}

/// A host-function closure. Receives the call's arguments already decoded from
/// JSON (`String`, `NSNumber` for `Int`/`Double`/`Bool`, `[Any]`, `[String: Any]`,
/// `NSNull` for `nil`) in declared order, and returns the result value to encode
/// against the declared return type (`nil` for a `Void` return). Throwing a
/// `TSwiftHostFunctionError` raises a catchable Swift error in the script.
public typealias TSwiftHostFunctionHandler = ([Any]) throws -> Any?

// MARK: - Registration API

extension TSwiftContext {
    /// Register (or replace, by name) a host function backed by `handler`.
    /// Available immediately to subsequent one-shot runs and SwiftUI compiles on
    /// this context. Returns the registered name on success.
    @discardableResult
    public func registerHostFunction(
        _ signature: TSwiftHostSignature,
        _ handler: @escaping TSwiftHostFunctionHandler
    ) throws -> String {
        let box = HostFunctionBox(handler)
        let resultJSON = signature.toJSON().withCString { cSignature -> String in
            let ptr = tswift_register_host_fn(
                handle,
                cSignature,
                { userdata, _, argsJSON, call in
                    guard let userdata, let argsJSON else { return }
                    let box = Unmanaged<HostFunctionBox>.fromOpaque(userdata)
                        .takeUnretainedValue()
                    let result = box.respond(argsJSON: String(cString: argsJSON))
                    result.withCString { tswift_host_respond(call, $0) }
                },
                Unmanaged.passUnretained(box).toOpaque()
            )
            guard let ptr else { return "" }
            defer { tswift_string_free(ptr) }
            return String(cString: ptr)
        }
        let outcome = HostFunctionBox.decodeRegistration(resultJSON)
        switch outcome {
        case let .success(name):
            // Retain the box for the context's lifetime; replacing a name
            // releases the prior box (the native side already dropped it).
            hostFunctionBoxes[name] = box
            return name
        case let .failure(message):
            throw TSwiftHostRegistrationError(message)
        }
    }

    /// Remove the host function named `name`; interpreted code can no longer
    /// call it. Releases the retained handler box. A no-op if never registered.
    public func removeHostFunction(named name: String) {
        name.withCString { tswift_remove_host_fn(handle, $0) }
        hostFunctionBoxes[name] = nil
    }
}

// MARK: - Handler box

/// Retained bridge between the C callback and a Swift closure: decodes the
/// argument JSON array, invokes the handler, and encodes the result (or a
/// thrown-error payload) back to JSON.
final class HostFunctionBox {
    private let handler: TSwiftHostFunctionHandler

    init(_ handler: @escaping TSwiftHostFunctionHandler) {
        self.handler = handler
    }

    /// Decode `argsJSON` (a JSON array), run the handler, and encode the result.
    func respond(argsJSON: String) -> String {
        let decoded = try? JSONSerialization.jsonObject(
            with: Data(argsJSON.utf8), options: [.fragmentsAllowed]
        )
        let args = decoded as? [Any] ?? []
        do {
            let result = try handler(args)
            return Self.encode(result ?? NSNull())
        } catch let error as TSwiftHostFunctionError {
            return Self.encodeThrown(error.message)
        } catch {
            return Self.encodeThrown("\(error)")
        }
    }

    /// Encode a result value to a JSON document (fragments allowed for scalars).
    private static func encode(_ value: Any) -> String {
        guard let data = try? JSONSerialization.data(
            withJSONObject: value, options: [.fragmentsAllowed]
        ), let json = String(data: data, encoding: .utf8) else {
            return "null"
        }
        return json
    }

    /// Encode a `{"$thrown":"<message>"}` payload raising a catchable error.
    private static func encodeThrown(_ message: String) -> String {
        encode(["$thrown": message])
    }

    /// The outcome of decoding the registration result JSON.
    enum RegistrationOutcome {
        case success(String)
        case failure(String)
    }

    private struct RegistrationEnvelope: Decodable {
        let ok: Bool
        let name: String?
        let error: String?
    }

    /// Decode the `{"ok","name","error"}` envelope from `tswift_register_host_fn`.
    static func decodeRegistration(_ json: String) -> RegistrationOutcome {
        guard let envelope = try? JSONDecoder().decode(
            RegistrationEnvelope.self, from: Data(json.utf8)
        ) else {
            let detail = json.isEmpty ? "tswift_register_host_fn returned null" : json
            return .failure(detail)
        }
        if envelope.ok, let name = envelope.name {
            return .success(name)
        }
        return .failure(envelope.error ?? "unknown registration failure")
    }
}
