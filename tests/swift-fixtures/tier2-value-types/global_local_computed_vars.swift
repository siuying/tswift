// expected-no-diagnostics
// oracle-gap: C msf does not accept accessor blocks on global/local variables

// TSPL "Global and Local Variables": computed accessors and observers are
// available to global and local variables, not just type members.

var stored = 10
var doubled: Int { stored * 2 }

var celsius = 0.0
var fahrenheit: Double {
  get { return celsius * 9 / 5 + 32 }
  set { celsius = (newValue - 32) * 5 / 9 }
}

var score = 0 {
  willSet { print("will set score to \(newValue)") }
  didSet { print("did set score from \(oldValue) to \(score)") }
}

var level = 1 {
  willSet(incoming) { print("incoming \(incoming)") }
  didSet(previous) { print("previous \(previous)") }
}

func localVars() {
  var local = 4
  var squared: Int { local * local }
  print(squared)
  var tracked = 0 {
    didSet { print("tracked \(oldValue) -> \(tracked)") }
  }
  tracked = 9
}
