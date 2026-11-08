// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Provides native on-device speech input for the macOS chat composer.

import AVFoundation
import Foundation
import Observation
import Speech

enum MacVoiceInputError: LocalizedError {
    case speechUnavailable
    case speechDenied
    case microphoneDenied

    var errorDescription: String? {
        switch self {
        case .speechUnavailable:
            return String(localized: "Speech recognition is unavailable for the current language.")
        case .speechDenied:
            return String(localized: "Speech recognition permission is required.")
        case .microphoneDenied:
            return String(localized: "Microphone permission is required.")
        }
    }
}

@MainActor
@Observable
final class MacVoiceInputService {
    var transcript = ""
    var isRecording = false
    var isStarting = false
    var errorMessage: String?
    private var generation = 0

    private let audioEngine = AVAudioEngine()
    private var recognitionRequest: SFSpeechAudioBufferRecognitionRequest?
    private var recognitionTask: SFSpeechRecognitionTask?
    private var hasInstalledInputTap = false

    func toggle() async {
        if isRecording {
            stop()
        } else {
            await start()
        }
    }

    func start() async {
        stop()
        let ticket = generation
        isStarting = true
        errorMessage = nil
        do {
            guard await requestSpeechAuthorization() else { throw MacVoiceInputError.speechDenied }
            guard generation == ticket else { return }
            guard await requestMicrophoneAuthorization() else { throw MacVoiceInputError.microphoneDenied }
            guard generation == ticket else { return }
            guard let recognizer = SFSpeechRecognizer(locale: Locale.current), recognizer.isAvailable else {
                throw MacVoiceInputError.speechUnavailable
            }

            let request = SFSpeechAudioBufferRecognitionRequest()
            request.shouldReportPartialResults = true
            request.requiresOnDeviceRecognition = recognizer.supportsOnDeviceRecognition
            recognitionRequest = request

            let inputNode = audioEngine.inputNode
            let format = inputNode.outputFormat(forBus: 0)
            inputNode.removeTap(onBus: 0)
            inputNode.installTap(onBus: 0, bufferSize: 1_024, format: format) { buffer, _ in
                request.append(buffer)
            }
            hasInstalledInputTap = true

            transcript = ""
            audioEngine.prepare()
            try audioEngine.start()
            isRecording = true
            isStarting = false

            recognitionTask = recognizer.recognitionTask(with: request) { [weak self] result, error in
                Task { @MainActor in
                    guard let self, self.generation == ticket else { return }
                    if let result {
                        self.transcript = result.bestTranscription.formattedString
                        if result.isFinal { self.stop() }
                    }
                    if let error {
                        self.errorMessage = error.localizedDescription
                        self.stop()
                    }
                }
            }
        } catch {
            guard generation == ticket else { return }
            errorMessage = error.localizedDescription
            stop()
        }
    }

    func stop() {
        generation &+= 1
        isStarting = false
        if audioEngine.isRunning {
            audioEngine.stop()
        }
        if hasInstalledInputTap {
            audioEngine.inputNode.removeTap(onBus: 0)
            hasInstalledInputTap = false
        }
        recognitionRequest?.endAudio()
        recognitionTask?.cancel()
        recognitionRequest = nil
        recognitionTask = nil
        isRecording = false
    }

    private func requestSpeechAuthorization() async -> Bool {
        switch SFSpeechRecognizer.authorizationStatus() {
        case .authorized: return true
        case .denied, .restricted: return false
        case .notDetermined:
            return await withCheckedContinuation { continuation in
                SFSpeechRecognizer.requestAuthorization { status in
                    continuation.resume(returning: status == .authorized)
                }
            }
        @unknown default: return false
        }
    }

    private func requestMicrophoneAuthorization() async -> Bool {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: return true
        case .denied, .restricted: return false
        case .notDetermined:
            return await AVCaptureDevice.requestAccess(for: .audio)
        @unknown default: return false
        }
    }
}
