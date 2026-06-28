// Double IEEE arithmetic, comparison and in-place mutation methods.
print((5.0).remainder(dividingBy: 3.0))
print((2.0).isEqual(to: 2.0), (2.0).isLess(than: 3.0), (3.0).isLessThanOrEqualTo(3.0))
print((10.0).distance(to: 13.0), (10.0).advanced(by: 5.0))

var s = 9.0
s.formSquareRoot()
print(s)

var r = 5.5
r.formTruncatingRemainder(dividingBy: 2.0)
print(r)

var fr = 5.0
fr.formRemainder(dividingBy: 3.0)
print(fr)

var rd = 2.6
rd.round()
print(rd)

var ap = 1.0
ap.addProduct(2.0, 3.0)
print(ap)
