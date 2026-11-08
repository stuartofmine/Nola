// SPDX-License-Identifier: Apache-2.0
import Foundation

enum IOSLocalRunAdmissionError: LocalizedError {
    case activeRun
    var errorDescription: String? {
        NSLocalizedString("chat.run.unresolved", value: "The previous task has not finished or its stop has not been confirmed. Resume or reconcile that task before sending again. Your draft has been kept.", comment: "Active Core Run blocks a new submission")
    }
}
