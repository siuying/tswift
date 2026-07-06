import SwiftUI
import XCTest

@testable import UiirRenderer

/// Fast, non-snapshot checks for the `.animation` / `.transition` decode +
/// apply path. We assert decode helpers produce non-nil values and that the
/// ModifierApply fold runs without crashing and yields a view.
@MainActor
final class AnimationDecodeTests: XCTestCase {
    // MARK: - Animation decode

    func testEaseInOutWithDurationDecodes() {
        let v = UiirValue.object(["$": .string("animation"),
                                  "kind": .string("easeInOut"),
                                  "duration": .number(0.3)])
        XCTAssertNotNil(AnimationDecode.animation(v))
    }

    func testBareCurveDecodes() {
        let v = UiirValue.object(["$": .string("animation"), "kind": .string("linear")])
        XCTAssertNotNil(AnimationDecode.animation(v))
    }

    func testSpringWithParamsDecodes() {
        let v = UiirValue.object(["$": .string("animation"),
                                  "kind": .string("spring"),
                                  "response": .number(0.5),
                                  "dampingFraction": .number(0.8)])
        XCTAssertNotNil(AnimationDecode.animation(v))
    }

    func testChainedModifiersDecode() {
        let v = UiirValue.object(["$": .string("animation"),
                                  "kind": .string("linear"),
                                  "delay": .number(0.1),
                                  "speed": .number(2),
                                  "repeat": .string("forever"),
                                  "autoreverses": .bool(false)])
        XCTAssertNotNil(AnimationDecode.animation(v))
    }

    func testRepeatCountDecodes() {
        let v = UiirValue.object(["$": .string("animation"),
                                  "kind": .string("easeIn"),
                                  "repeat": .number(3),
                                  "autoreverses": .bool(true)])
        XCTAssertNotNil(AnimationDecode.animation(v))
    }

    func testNullAnimationIsNil() {
        XCTAssertNil(AnimationDecode.animation(.null))
    }

    // MARK: - Transition decode (recursive)

    func testSimpleTransitionsDecode() {
        for type in ["opacity", "identity", "slide", "scale"] {
            let t = AnimationDecode.transition(.object(["type": .string(type)]))
            _ = t  // AnyTransition is opaque; just confirm no crash.
        }
    }

    func testScaleFactoryWithAnchor() {
        let v = UiirValue.object(["type": .string("scale"),
                                  "scale": .number(0.5),
                                  "anchor": .string("topLeading")])
        _ = AnimationDecode.transition(v)
    }

    func testMoveAndPushEdges() {
        _ = AnimationDecode.transition(.object(["type": .string("move"),
                                                "edge": .string("leading")]))
        _ = AnimationDecode.transition(.object(["type": .string("push"),
                                                "edge": .string("bottom")]))
    }

    func testCombinedAndAsymmetricRecurse() {
        let combined = UiirValue.object(["type": .string("combined"),
                                         "transitions": .array([
                                             .object(["type": .string("opacity")]),
                                             .object(["type": .string("slide")]),
                                         ])])
        _ = AnimationDecode.transition(combined)

        let asym = UiirValue.object(["type": .string("asymmetric"),
                                     "insertion": .object(["type": .string("scale")]),
                                     "removal": .object(["type": .string("opacity")])])
        _ = AnimationDecode.transition(asym)
    }

    func testUnknownTransitionFallsBackToIdentity() {
        _ = AnimationDecode.transition(.object(["type": .string("nope")]))
        _ = AnimationDecode.transition(.null)
    }

    // MARK: - ModifierApply integration (no snapshot)

    func testAnimationModifierAppliesWithoutCrash() {
        let mod = UiirModifier(name: "animation", value: .object([
            "animation": .object(["$": .string("animation"),
                                  "kind": .string("easeInOut"),
                                  "duration": .number(0.3)]),
            "value": .bool(true),
        ]))
        let out = ModifierApply.apply([mod], to: AnyView(Color.clear))
        XCTAssertNotNil(out)
    }

    func testDeprecatedAnimationFormAppliesWithoutCrash() {
        let mod = UiirModifier(name: "animation", value: .object([
            "animation": .object(["$": .string("animation"), "kind": .string("linear")]),
        ]))
        let out = ModifierApply.apply([mod], to: AnyView(Color.clear))
        XCTAssertNotNil(out)
    }

    func testTransitionModifierAppliesWithoutCrash() {
        let mod = UiirModifier(name: "transition", value: .object([
            "type": .string("asymmetric"),
            "insertion": .object(["type": .string("scale"), "scale": .number(0.5)]),
            "removal": .object(["type": .string("opacity")]),
        ]))
        let out = ModifierApply.apply([mod], to: AnyView(Color.clear))
        XCTAssertNotNil(out)
    }
}
