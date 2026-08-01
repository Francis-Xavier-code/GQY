#!/usr/bin/env swift
// GQY 本地语音识别工具：把音频文件转成文字（SFSpeechRecognizer，离线免费）。
// 不依赖任何模型 API 额度；与 vision-tool.swift 同一模式。
// 用法: swift speech-tool.swift <音频文件> [zh-Hans|en-US]
//
// 重要：macOS 的语音识别是 TCC 敏感权限，裸 swift 脚本无法自动获得授权
// （会因隐私违规崩溃）。本工具检测到权限不可用时输出可读指引而非崩溃；
// 若要真正启用，需要把 GQY 的 STT 能力放进带 bundle 身份的 App（菜单栏 App）。
import Speech
import Foundation

let arguments = CommandLine.arguments
guard arguments.count >= 2 else {
    print("{\"error\":\"usage: speech-tool <audio> [locale]\"}")
    exit(1)
}
let audioPath = arguments[1]
let localeArg = arguments.count >= 3 ? arguments[2] : "zh-Hans"

func fail(_ message: String) -> Never {
    print("{\"ok\":false,\"error\":\"\(message)\"}")
    exit(0)
}

// 语音识别权限（本地离线识别也需要显式授权）
var authStatus = SFSpeechRecognizer.authorizationStatus()
if authStatus == .notDetermined {
    // 请求授权；裸脚本场景通常会因 TCC 隐私违规崩溃或被拒，
    // 这里兜底：请求后短暂等待再查状态
    let sem = DispatchSemaphore(value: 0)
    SFSpeechRecognizer.requestAuthorization { status in
        authStatus = status
        sem.signal()
    }
    _ = sem.wait(timeout: .now() + 3)
}

switch authStatus {
case .authorized:
    break
case .denied:
    fail("speech recognition permission denied. 请到 系统设置 → 隐私与安全性 → 语音识别 允许 GQY/终端 使用")
case .restricted:
    fail("speech recognition is restricted on this system")
case .notDetermined:
    fail("speech recognition permission not granted. 裸脚本无法自动授权，需要把 STT 集成进带 bundle 的 App")
@unknown default:
    fail("unknown speech recognition authorization status")
}

guard let recognizer = SFSpeechRecognizer(locale: Locale(identifier: localeArg)) else {
    fail("unsupported locale: \(localeArg)")
}
guard recognizer.isAvailable else {
    fail("speech recognizer unavailable (network or system limitation)")
}

let url = URL(fileURLWithPath: audioPath)
guard FileManager.default.fileExists(atPath: audioPath) else {
    fail("audio file not found: \(audioPath)")
}

let semaphore = DispatchSemaphore(value: 0)
var transcript = ""
var recognitionError: String?

let request = SFSpeechURLRecognitionRequest(url: url)
request.shouldReportPartialResults = false
recognizer.recognitionTask(with: request) { result, error in
    if let error = error {
        recognitionError = error.localizedDescription
    } else if let result = result, result.isFinal {
        transcript = result.bestTranscription.formattedString
    }
    semaphore.signal()
}

_ = semaphore.wait(timeout: .now() + 120)

if let recognitionError = recognitionError {
    print("{\"ok\":false,\"error\":\"\(recognitionError)\"}")
} else {
    print("{\"ok\":true,\"text\":\"\(transcript)\",\"locale\":\"\(localeArg)\"}")
}
