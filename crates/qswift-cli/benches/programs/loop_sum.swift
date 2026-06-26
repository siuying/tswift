// Tight-loop workload: arithmetic + compound assignment in a hot while loop.
// Stresses environment lookups, integer ops, and branch evaluation.
var sum = 0
var i = 1
while i <= 200000 {
    sum &+= i
    i += 1
}
print(sum)
