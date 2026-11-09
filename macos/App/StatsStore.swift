import Foundation

/// Usage days and per-model accuracy read from the IME's `hooks/` blobs.
final class StatsStore: ObservableObject {
    @Published var days: [UsageDay] = []
    @Published var models: [String: ModelAccuracy] = [:]

    func load(dataDir: URL) {
        let fm = FileManager.default
        let usage = dataDir.appendingPathComponent("blobs/hooks/usage")
        let names = (try? fm.contentsOfDirectory(atPath: usage.path)) ?? []
        days = StatsLogic.lastDays(StatsLogic.recentDateNames(names).compactMap { name in
            (try? Data(contentsOf: usage.appendingPathComponent(name))).flatMap(UsageData.parse)
        })
        let stats = dataDir.appendingPathComponent("blobs/hooks/model_stats")
        var out: [String: ModelAccuracy] = [:]
        for kind in (try? fm.contentsOfDirectory(atPath: stats.path)) ?? [] where !kind.hasPrefix(".") {
            if let acc = (try? Data(contentsOf: stats.appendingPathComponent(kind))).flatMap(ModelAccuracy.parse) {
                out[kind] = acc
            }
        }
        models = out
    }
}
