enum Planet: Int, CaseIterable {
    case mercury = 1, venus, earth
    func describe() -> String { return "planet \(rawValue)" }
}
print(Planet.earth.rawValue)
print(Planet.venus.describe())
for p in Planet.allCases { print(p.rawValue) }
