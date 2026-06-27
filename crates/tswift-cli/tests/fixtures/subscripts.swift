let nums = [10, 20, 30, 40]
print(nums[0], nums[3])
struct Matrix {
    var flat: [Int]
    subscript(_ i: Int) -> Int { return flat[i] }
}
let m = Matrix(flat: [7, 8, 9])
print(m[1])
let word = "Swift"
print(word[0])
