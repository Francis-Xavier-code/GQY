#!/usr/bin/env swift
// GQY 本地视觉工具：OCR 文字识别 + 图像分类标签（Apple Vision，离线免费）。
// 不依赖任何模型 API 额度；模型超额时 GQY 用它兜底看图。
// 用法: swift vision-tool.swift <图片路径> [ocr|describe|all]
import Vision
import AppKit
import Foundation

let arguments = CommandLine.arguments
guard arguments.count >= 2 else {
    print("{\"error\":\"usage: vision-tool <image> [ocr|describe|all]\"}")
    exit(1)
}
let imagePath = arguments[1]
let mode = arguments.count >= 3 ? arguments[2] : "all"
guard let image = NSImage(contentsOfFile: imagePath),
      let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else {
    print("{\"error\":\"cannot load image: \(imagePath)\"}")
    exit(1)
}

var result: [String: Any] = [:]

if mode == "all" || mode == "ocr" {
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.recognitionLanguages = ["zh-Hans", "en-US"]
    let handler = VNImageRequestHandler(cgImage: cgImage)
    try? handler.perform([request])
    let texts = (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
    result["ocr"] = texts
}

if mode == "all" || mode == "describe" {
    let classify = VNClassifyImageRequest()
    let handler = VNImageRequestHandler(cgImage: cgImage)
    try? handler.perform([classify])
    let labels = (classify.results ?? [])
        .filter { $0.confidence > 0.3 }
        .sorted { $0.confidence > $1.confidence }
        .prefix(10)
        .map { "\($0.identifier)\(String(format: "%.0f%%", $0.confidence * 100))" }
    result["labels"] = Array(labels)

    let rectangles = VNDetectRectanglesRequest()
    try? handler.perform([rectangles])
    let objects = (rectangles.results ?? [])
        .filter { $0.confidence > 0.4 }
        .map { "\(String(format: "%.0f%%", $0.confidence * 100)) \(Int($0.boundingBox.width * 100))x\(Int($0.boundingBox.height * 100))" }
    result["objects"] = Array(objects)
}

let json = try? JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted])
print(String(data: json ?? Data(), encoding: .utf8) ?? "{}")
