import Foundation
import XCTest

final class MacChatLocalizationContractTests: XCTestCase {
    func testLocalExecutionRecoveryDoesNotClaimCancellationOrSuccess() {
        let chinese = Locale(identifier: "zh-Hans")
        XCTAssertEqual(String(localized: "Local execution connection interrupted. The result is not yet confirmed.",
                              table: "Chat", bundle: .main, locale: chinese),
                       "本地执行连接已中断，结果尚未确认。")
        XCTAssertEqual(String(localized: "Check execution status", table: "Chat", bundle: .main, locale: chinese),
                       "检查执行状态")
    }

    func testConversationInspectorUsesChatCatalogTranslations() throws {
        let simplifiedChinese = Locale(identifier: "zh-Hans")

        XCTAssertEqual(
            String(
                localized: "Conversation Details",
                table: "Chat",
                bundle: .main,
                locale: simplifiedChinese
            ),
            "对话详情"
        )
        XCTAssertEqual(
            String(
                localized: "Applies to this conversation",
                table: "Chat",
                bundle: .main,
                locale: simplifiedChinese
            ),
            "仅对当前对话有效"
        )
        XCTAssertEqual(
            String(
                localized: "Knowledge",
                table: "Chat",
                bundle: .main,
                locale: simplifiedChinese
            ),
            "知识"
        )
        XCTAssertEqual(
            String(
                format: String(
                    localized: "Context %@ / %@",
                    bundle: .main,
                    locale: simplifiedChinese
                ),
                "~5k",
                "128k"
            ),
            "上下文 ~5k / 128k"
        )
        XCTAssertEqual(
            String(localized: "Choose…", bundle: .main, locale: simplifiedChinese),
            "选择…"
        )
        XCTAssertEqual(
            String(
                localized: "Files",
                table: "Chat",
                bundle: .main,
                locale: simplifiedChinese
            ),
            "文件"
        )
        XCTAssertEqual(
            String(
                localized: "Browser",
                table: "Chat",
                bundle: .main,
                locale: simplifiedChinese
            ),
            "浏览器"
        )
        let germanResource = try XCTUnwrap(
            Bundle.main.url(
                forResource: "Chat",
                withExtension: "strings",
                subdirectory: nil,
                localization: "de"
            )
        )
        let germanData = try Data(contentsOf: germanResource)
        let germanStrings = try XCTUnwrap(
            PropertyListSerialization.propertyList(from: germanData, format: nil) as? [String: String]
        )

        XCTAssertEqual(germanStrings["Conversation Details"], "Gesprächsdetails")
        XCTAssertEqual(germanStrings["Applies to this conversation"], "Gilt für diese Unterhaltung")
    }
}
