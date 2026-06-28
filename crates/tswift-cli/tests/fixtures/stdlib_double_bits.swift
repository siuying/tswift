// Double IEEE-754 decomposition: bit pattern, exponent, significand, binade.
print((3.0).bitPattern)
print((3.0).exponent, (0.75).exponent, (1.0).exponent)
print((3.0).significand, (0.75).significand, (1.0).significand)
print((3.0).binade, (0.75).binade, (-3.0).binade)
print((1.0).exponentBitPattern, (1.0).significandBitPattern)
print((1.0).isCanonical, (3.0).isSignalingNaN)
print((2.0).hashValue == (2.0).hashValue, (0.0).hashValue == (-0.0).hashValue)
