import Foundation

/// Loads UIIR + patch fixtures from `tests/swiftui-fixtures/`, located by walking
/// up from a caller source file so the fixtures stay single-sourced in the repo.
public enum FixtureLoader {
    /// Find the repo's `tests/swiftui-fixtures/` directory from a source path.
    public static func fixturesDir(from filePath: String = #filePath) -> URL {
        var dir = URL(fileURLWithPath: filePath).deletingLastPathComponent()
        let fm = FileManager.default
        for _ in 0..<12 {
            let candidate = dir.appendingPathComponent("tests/swiftui-fixtures")
            if fm.fileExists(atPath: candidate.path) { return candidate }
            dir = dir.deletingLastPathComponent()
        }
        fatalError("Could not locate tests/swiftui-fixtures from \(filePath)")
    }

    /// Decode `<name>.uiir.json` into a `UiirNode`.
    public static func loadUiir(_ name: String, from filePath: String = #filePath) throws -> UiirNode {
        let url = fixturesDir(from: filePath).appendingPathComponent("\(name).uiir.json")
        let data = try Data(contentsOf: url)
        return try JSONDecoder().decode(UiirNode.self, from: data)
    }

    /// Decode `<name>.patches.json` into per-event patch batches, if present.
    public static func loadPatches(_ name: String, from filePath: String = #filePath) throws -> [[Patch]] {
        let url = fixturesDir(from: filePath).appendingPathComponent("\(name).patches.json")
        guard FileManager.default.fileExists(atPath: url.path) else { return [] }
        let data = try Data(contentsOf: url)
        return try JSONDecoder().decode([[Patch]].self, from: data)
    }

    /// All fixture base names (those with a `.uiir.json`).
    public static func allFixtures(from filePath: String = #filePath) -> [String] {
        let dir = fixturesDir(from: filePath)
        let names = (try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? []
        return names
            .filter { $0.hasSuffix(".uiir.json") }
            .map { String($0.dropLast(".uiir.json".count)) }
            .sorted()
    }
}
