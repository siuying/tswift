// Double.sign (FloatingPointSign), significandWidth, and the quiet/signaling
// NaN type constants.
print((-3.5).sign == .minus)
print((3.5).sign == .plus)
print((-0.0).sign == .minus)
print((1.0).significandWidth)
print((1.5).significandWidth)
print((1.25).significandWidth)
print((0.0).significandWidth)
print(Double.infinity.significandWidth)
print(Double.quietNaN.isNaN)
print(Double.signalingNaN.isSignalingNaN)
