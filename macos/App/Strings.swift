/// Every user-facing string of the companion app (Vietnamese).
enum Strings {
    // Sections and menus
    static let overview = "Tổng quan"
    static let statistics = "Thống kê"
    static let settings = "Cài đặt"
    static let log = "Nhật ký"
    static let view = "Xem"
    static let refresh = "Làm mới"

    // Install state
    static let installState = "Bộ gõ"
    static let installed = "Đã cài đặt"
    static let notInstalled = "Chưa cài đặt"
    static let installFailed = "Cài đặt thất bại"
    static let retry = "Thử lại"
    static let version = "Phiên bản"

    // Input source
    static let typingInABC = "NeuralIMK đã được bật nhưng bạn đang gõ bằng bộ gõ khác (ví dụ ABC)."
    static let selectNeuralIMK = "Chọn NeuralIMK"
    static let imeSelected = "NeuralIMK đang được dùng"

    // Model
    static let activeModel = "Model đang dùng"
    static let recentAccuracy = "Độ chính xác gần đây"
    static let lifetimeAccuracy = "Độ chính xác từ đầu"
    static let noWordsYet = "Chưa có dữ liệu — hãy gõ vài từ"
    static func wordsCount(_ n: Int) -> String { "\(n) từ" }

    // Cheat sheet
    static let cheatSheet = "Phím tắt"
    static let doubleShift = "Shift hai lần"
    static let doubleShiftAction = "sửa từ vừa gõ (đổi Việt ↔ Anh)"
    static let doubleShiftAgain = "Shift hai lần lần nữa"
    static let doubleShiftAgainAction = "hoàn tác"
    static let commandPrefix = "/ @ ! đầu từ"
    static let commandPrefixAction = "gõ lệnh (không thêm dấu)"
    static let reviewGuide = "Xem lại hướng dẫn"

    // Placeholders replaced by later screens
    static let comingSoon = "Sắp có"
    static let onboardingTitle = "Chào mừng đến với NeuralIMK"
    static let close = "Đóng"
}
