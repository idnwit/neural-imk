/// Every user-facing string of the companion app (Vietnamese).
enum Strings {
    // Sections and menus
    static let overview = "Tổng quan"
    static let statistics = "Thống kê"
    static let settings = "Cài đặt"
    static let log = "Nhật ký"
    static let caretLab = "Thử sửa từ"
    static let view = "Xem"
    static let refresh = "Làm mới"

    // Caret-flip simulator (ADR 0015)
    static let labIntro = "Sửa một từ đã gõ xong: đưa con trỏ vào từ sai rồi bấm Shift hai lần. Từ đổi giữa tiếng Việt và tiếng Anh ngay tại chỗ, con trỏ giữ nguyên. Hai ô dưới đây dùng bộ gõ thật, nên cần chọn NeuralIMK làm bộ gõ."
    static let labEditorTitle = "Ô soạn thảo (như VS Code, Zed, TextEdit)"
    static let labEditorSteps = [
        "Gõ một câu có từ bị đổi nhầm, ví dụ gõ \"chajy test laij\" (test thành \"tét\").",
        "Bấm vào giữa \"tét\", hoặc ngay sau nó, hoặc bôi đen nó.",
        "Bấm Shift hai lần: \"tét\" thành \"test\". Bấm lần nữa để đổi lại.",
    ]
    static let labTerminalTitle = "Terminal giả lập (như Claude Code, Codex trong terminal)"
    static let labTerminalSteps = [
        "Bấm vào ô đen rồi gõ một dòng, ví dụ \"fix test nayf\".",
        "Dùng ← → để đưa con trỏ vào từ sai (không bấm chuột: chuột làm bộ gõ mất dấu dòng).",
        "Bấm Shift hai lần: bộ gõ xoá từ bằng DEL rồi gõ lại, như trong terminal thật.",
    ]
    static let labTrace = "Terminal nhận được (mới nhất ở trên):"
    static let labTerminalNote = "Bộ gõ chỉ theo được dòng gõ từ lần Enter gần nhất; sau ↑ ↓ Tab Esc hay click chuột, bấm Enter để bắt đầu lại."
    static let labTerminalPlaceholder = "bấm vào đây rồi gõ"
    static let labClear = "Xoá"

    // Install state
    static let installState = "Bộ gõ"
    static let installed = "Đã cài đặt"
    static let notInstalled = "Chưa cài đặt"
    static let installFailed = "Cài đặt thất bại"
    static let retry = "Thử lại"
    static let reinstall = "Cài lại NeuralIMK"
    static let uninstalledByUser = "Bạn đã gỡ NeuralIMK."
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

    // Statistics and log
    static let noDataYet = "Chưa có dữ liệu — hãy gõ vài từ"
    static let showAsTable = "Hiện dạng bảng"
    static let wordsPerDay = "Từ mỗi ngày"
    static let correctionsPerDay = "Sửa mỗi ngày"
    static let modelAccuracy = "Độ chính xác model"
    static let perApp = "Theo ứng dụng"
    static let vietnamese = "Tiếng Việt"
    static let english = "Tiếng Anh"
    static let date = "Ngày"
    static let words = "Từ"
    static let corrections = "Lần sửa"
    static let model = "Model"
    static let app = "Ứng dụng"
    static let filter = "Lọc"
    static let openDataFolder = "Mở thư mục dữ liệu"
    static let noLog = "Chưa có nhật ký"

    // Placeholders replaced by later screens
    static let comingSoon = "Sắp có"
    static let onboardingTitle = "Chào mừng đến với NeuralIMK"
    static let close = "Đóng"
}

// Settings
extension Strings {
    static let hotkeySection = "Phím tắt sửa từ"
    static let hotkeyPicker = "Phím tắt"
    static let presetDoubleShift = "Double Shift"
    static let presetCtrlSpace = "Ctrl+Space"
    static let presetBoth = "Cả hai"
    static let presetOff = "Tắt"
    static let noHotkeyUseMenu = "Không có phím tắt: dùng menu của bộ gõ trên thanh menu để sửa từ."
    static let systemShortcutWarning = "macOS đang dùng Ctrl+Space để chuyển bộ gõ, nên phím này sẽ không tới NeuralIMK."
    static let openKeyboardShortcuts = "Mở Phím tắt bàn phím"
    static let tryNow = "Thử ngay"
    static let tryNowPlaceholder = "Gõ thử ở đây"
    static let showPerAppUsage = "Hiện thống kê theo ứng dụng"
    static let developerMode = "Chế độ nhà phát triển"
    static let data = "Dữ liệu"
    static let deleteUsage = "Xoá dữ liệu thống kê"
    static let deleteUsageConfirm = "Xoá toàn bộ dữ liệu thống kê?"
    static let deleteUsageMessage = "Chỉ xoá thống kê sử dụng; dữ liệu học của model được giữ nguyên. Bộ gõ sẽ được khởi động lại."
    static let delete = "Xoá"
    static let cancel = "Huỷ"
    static let uninstall = "Gỡ cài đặt NeuralIMK…"
    static let uninstallConfirm = "Gỡ cài đặt NeuralIMK?"
    static let uninstallMessage = "Bộ gõ sẽ bị xoá khỏi thư mục Input Methods."
    static let deleteLearnedData = "Xoá cả dữ liệu học"
    static let uninstallAction = "Gỡ cài đặt"
    static let uninstallFailed = "Gỡ cài đặt thất bại"
    static let removeFromInputSources = "Đã gỡ NeuralIMK. Hãy xoá nó khỏi danh sách Nguồn đầu vào trong Cài đặt hệ thống."
    static let openKeyboardSettings = "Mở cài đặt Bàn phím"
    static let configWriteFailed = "Không ghi được config"
}

// Onboarding
extension Strings {
    static let next = "Tiếp"
    static let back = "Quay lại"
    static let start = "Bắt đầu dùng"
    static func stepOf(_ n: Int, _ total: Int) -> String { "Bước \(n)/\(total)" }
    static let stepInstall = "Cài đặt"
    static let stepSelect = "Chọn NeuralIMK"
    static let stepPractice = "Luyện tập"
    static let stepDone = "Xong"
    static let statsStayLocal = "Thống kê chỉ lưu trên máy của bạn, không gửi đi đâu."
    static let enableNeuralIMK = "Bật NeuralIMK"
    static let globeTip = "Mẹo: nhấn phím Globe (Fn) để đổi bộ gõ nhanh."
    static let capsLockTip = "Mẹo: trong Cài đặt Bàn phím > Nguồn đầu vào, bật \"Use Caps Lock to switch to and from ABC\" nếu muốn dùng Caps Lock."
    static let practiceInstruction = "Gõ câu dưới đây, cố ý gõ sai một từ rồi nhấn Shift hai lần để sửa."
    static let practicePlaceholder = "Gõ câu mẫu vào đây"
    static let practiceDone = "Đã sửa từ thành công"
    static let practicePrompts = [
        "chao ban, hom nay toi hoc code",
        "toi dang go tieng Viet voi NeuralIMK",
        "cuoi tuan nay chung ta di choi nhe",
    ]
}
