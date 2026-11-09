import Charts
import SwiftUI

struct StatisticsView: View {
    @EnvironmentObject var model: AppModel
    @StateObject private var store = StatsStore()
    @State private var asTable = false

    private struct WordPoint: Identifiable {
        let id: String
        let date: String
        let series: String
        let words: Int
    }

    private struct AccuracyPoint: Identifiable {
        let id: String
        let model: String
        let series: String
        let percent: Double
    }

    private var wordPoints: [WordPoint] {
        store.days.flatMap { d in
            [WordPoint(id: d.date + "vi", date: d.date, series: Strings.vietnamese, words: d.total.vi),
             WordPoint(id: d.date + "en", date: d.date, series: Strings.english, words: d.total.en)]
        }
    }

    private var accuracyPoints: [AccuracyPoint] {
        store.models.sorted { $0.key < $1.key }.flatMap { kind, acc -> [AccuracyPoint] in
            var pts: [AccuracyPoint] = []
            if let r = acc.recent { pts.append(AccuracyPoint(id: kind + "r", model: kind, series: Strings.recentAccuracy, percent: r * 100)) }
            if let l = acc.lifetime { pts.append(AccuracyPoint(id: kind + "l", model: kind, series: Strings.lifetimeAccuracy, percent: l * 100)) }
            return pts
        }
    }

    private var apps: [(app: String, words: Int)] { UsageData.topApps(store.days, limit: 8) }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                Toggle(Strings.showAsTable, isOn: $asTable)
                if store.days.isEmpty && accuracyPoints.isEmpty {
                    Text(Strings.noDataYet).foregroundStyle(.secondary)
                } else {
                    wordsSection
                    correctionsSection
                    if !accuracyPoints.isEmpty { accuracySection }
                    if model.showPerAppUsage && !apps.isEmpty { appsSection }
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .navigationTitle(Strings.statistics)
        .onAppear { store.load(dataDir: model.dataDir) }
        .onChange(of: model.dataRevision) { _ in store.load(dataDir: model.dataDir) }
    }

    private func block<C: View>(_ title: String, summary: String?, @ViewBuilder content: () -> C) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title).font(.headline)
            content()
                .accessibilityElement(children: asTable ? .contain : .ignore)
                .accessibilityLabel(summary ?? title)
        }
    }

    private func table(_ header: [String], _ rows: [[String]]) -> some View {
        Grid(alignment: .leading, horizontalSpacing: 20, verticalSpacing: 4) {
            GridRow { ForEach(header, id: \.self) { Text($0).bold() } }
            ForEach(Array(rows.enumerated()), id: \.offset) { _, r in
                GridRow { ForEach(Array(r.enumerated()), id: \.offset) { _, c in Text(c).monospacedDigit() } }
            }
        }
    }

    private var wordsSection: some View {
        block(Strings.wordsPerDay, summary: StatsLogic.wordsSummary(store.days)) {
            if asTable {
                table([Strings.date, Strings.vietnamese, Strings.english],
                      store.days.map { [$0.date, "\($0.total.vi)", "\($0.total.en)"] })
            } else {
                Chart(wordPoints) { p in
                    BarMark(x: .value(Strings.date, p.date), y: .value(Strings.words, p.words))
                        .foregroundStyle(by: .value("", p.series))
                }
                .frame(height: 200)
            }
        }
    }

    private var correctionsSection: some View {
        block(Strings.correctionsPerDay, summary: StatsLogic.correctionsSummary(store.days)) {
            if asTable {
                table([Strings.date, Strings.corrections], store.days.map { [$0.date, "\($0.total.corrected)"] })
            } else {
                Chart(store.days, id: \.date) { d in
                    LineMark(x: .value(Strings.date, d.date), y: .value(Strings.corrections, d.total.corrected))
                    PointMark(x: .value(Strings.date, d.date), y: .value(Strings.corrections, d.total.corrected))
                }
                .frame(height: 160)
            }
        }
    }

    private var accuracySection: some View {
        let summary = store.models.sorted { $0.key < $1.key }
            .map { "\($0.key): \(StatsLogic.percent($0.value.recent)) gần đây, \(StatsLogic.percent($0.value.lifetime)) từ đầu" }
            .joined(separator: "; ")
        return block(Strings.modelAccuracy, summary: summary) {
            if asTable {
                table([Strings.model, Strings.recentAccuracy, Strings.lifetimeAccuracy],
                      store.models.sorted { $0.key < $1.key }
                          .map { [$0.key, StatsLogic.percent($0.value.recent), StatsLogic.percent($0.value.lifetime)] })
            } else {
                Chart(accuracyPoints) { p in
                    BarMark(x: .value(Strings.model, p.model), y: .value("%", p.percent))
                        .foregroundStyle(by: .value("", p.series))
                        .position(by: .value("", p.series))
                }
                .chartYScale(domain: 0...100)
                .frame(height: 180)
            }
        }
    }

    private var appsSection: some View {
        let summary = apps.map { "\($0.app): \($0.words) từ" }.joined(separator: "; ")
        return block(Strings.perApp, summary: summary) {
            if asTable {
                table([Strings.app, Strings.words], apps.map { [$0.app, "\($0.words)"] })
            } else {
                Chart(apps, id: \.app) { a in
                    BarMark(x: .value(Strings.words, a.words), y: .value(Strings.app, a.app))
                }
                .frame(height: CGFloat(max(1, apps.count)) * 28 + 30)
            }
        }
    }
}
