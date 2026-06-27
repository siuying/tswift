// @globalActor declarations and global-actor / @preconcurrency annotations are
// accepted and run on the cooperative single-threaded executor.
@globalActor
actor DataActor {
    static let shared = DataActor()
}

@DataActor
func compute() -> Int { 42 }

@DataActor
struct Config {
    var level = 3
    func describe() -> String { "level \(level)" }
}

@preconcurrency import Foundation

print(compute())
print(Config().describe())
print(Config(level: 7).level)
