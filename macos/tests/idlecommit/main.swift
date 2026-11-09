import Foundation

// Run with `make swift-test` (compiles Debouncer.swift and IdleCommit.swift).
var failures = 0
func check(_ ok: Bool, _ name: String) {
    if !ok { failures += 1; print("FAIL: \(name)") }
}

func settle(_ secs: TimeInterval) {
    RunLoop.main.run(until: Date(timeIntervalSinceNow: secs))
}

do {
    let t = IdleCommit()
    var fired = 0
    t.update(secs: 0.05, marked: true) { fired += 1 }
    check(t.isArmed, "armed while text is marked")
    settle(0.2)
    check(fired == 1, "fires once after the silence")
    check(!t.isArmed, "disarmed after firing")
}
do {
    // Every key re-arms: typing faster than the timeout never fires it.
    let t = IdleCommit()
    var fired = 0
    for _ in 0..<5 {
        t.update(secs: 0.1, marked: true) { fired += 1 }
        settle(0.04)
    }
    check(fired == 0, "keys within the timeout postpone the commit")
    settle(0.25)
    check(fired == 1, "fires after the last key")
}
do {
    let t = IdleCommit()
    var fired = 0
    t.update(secs: 0, marked: true) { fired += 1 }
    check(!t.isArmed, "0 turns the timer off")
    t.update(secs: 0.05, marked: false) { fired += 1 }
    check(!t.isArmed, "nothing marked: no timer")
    settle(0.15)
    check(fired == 0, "never fired")
}
do {
    let t = IdleCommit()
    var fired = 0
    t.update(secs: 0.05, marked: true) { fired += 1 }
    t.update(secs: 0.05, marked: false) { fired += 1 }
    settle(0.15)
    check(fired == 0, "a commit (nothing marked any more) cancels the timer")
    t.update(secs: 0.05, marked: true) { fired += 1 }
    t.cancel()
    settle(0.15)
    check(fired == 0, "cancel on deactivate")
}
print(failures == 0 ? "ok" : "\(failures) failure(s)")
exit(failures == 0 ? 0 : 1)
