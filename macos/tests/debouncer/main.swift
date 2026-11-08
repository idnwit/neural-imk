import Foundation

// Run with `make swift-test` (compiles Debouncer.swift with this file).
var failures = 0
func check(_ ok: Bool, _ name: String) {
    if !ok { failures += 1; print("FAIL: \(name)") }
}

func settle(_ secs: TimeInterval) {
    RunLoop.main.run(until: Date(timeIntervalSinceNow: secs))
}

do {
    let d = Debouncer()
    var ran: [Int] = []
    d.schedule(after: 0.05) { ran.append(1) }
    d.schedule(after: 0.05) { ran.append(2) }
    settle(0.2)
    check(ran == [2], "a newer schedule replaces the older")
    check(!d.isPending, "not pending after running")
}
do {
    let d = Debouncer()
    var ran = 0
    d.schedule(after: 0.05) { ran += 1 }
    d.cancel()
    settle(0.2)
    check(ran == 0, "cancel drops the action")
}
do {
    let d = Debouncer()
    var ran = 0
    d.schedule(after: 0.05) { ran += 1 }
    check(d.flush { ran += 10 }, "flush reports pending")
    settle(0.2)
    check(ran == 10, "flush runs now and drops the delayed action")
    check(!d.flush { ran += 100 }, "flush with nothing pending is a no-op")
    check(ran == 10, "no-op flush does not run")
}
print(failures == 0 ? "ok" : "\(failures) failure(s)")
exit(failures == 0 ? 0 : 1)
