// Int masking-shift compound assignments (&<<=, &>>=): shift amounts are
// reduced modulo the bit width, and the result wraps within the type.
var a = 1
a &<<= 4
print(a)
var b = 256
b &>>= 2
print(b)
var c = 1
c &<<= 64
print(c)
