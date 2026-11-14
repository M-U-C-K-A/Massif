import CoreGraphics
import Foundation
import ImageIO
import QuickLookThumbnailing

/// Shows the map of a Massif map as the icon of its .massif file in the Finder, as WorldPainter's fork does for .world
/// files and Blender for .blend files.
///
/// Massif keeps a JPEG picture of the map in a section of kind "THMB" of the file (see massif-format): the header
/// gives where the directory of sections is, the directory where the picture is. Nothing else is read. Files saved by
/// older versions have none; the Finder then shows the usual document icon.
class ThumbnailProvider: QLThumbnailProvider {
    override func provideThumbnail(for request: QLFileThumbnailRequest, _ handler: @escaping (QLThumbnailReply?, Error?) -> Void) {
        guard let jpeg = readPicture(request.fileURL),
              let source = CGImageSourceCreateWithData(jpeg as CFData, nil),
              let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else {
            handler(nil, CocoaError(.fileReadCorruptFile))
            return
        }
        let width = CGFloat(image.width), height = CGFloat(image.height)
        let scale = min(request.maximumSize.width / width, request.maximumSize.height / height)
        let size = CGSize(width: max((width * scale).rounded(), 1), height: max((height * scale).rounded(), 1))
        handler(QLThumbnailReply(contextSize: size) { context in
            context.interpolationQuality = .high
            context.draw(image, in: CGRect(origin: .zero, size: size))
            return true
        }, nil)
    }

    /// The picture of the map: the payload of the "THMB" section, if any.
    private func readPicture(_ url: URL) -> Data? {
        guard let file = try? FileHandle(forReadingFrom: url) else {
            return nil
        }
        defer {
            try? file.close()
        }
        // The header: "MASSIF\u{1a}\n", then where the directory is and how long it is
        let header = [UInt8](file.readData(ofLength: 64))
        guard header.count == 64, header[0..<8] == [0x4d, 0x41, 0x53, 0x53, 0x49, 0x46, 0x1a, 0x0a] else {
            return nil
        }
        let directoryOffset = little(header, 0x10, 8)
        let directoryLength = Int(little(header, 0x18, 4))
        guard directoryLength % 32 == 0, directoryLength <= 1 << 24,
              (try? file.seek(toOffset: directoryOffset)) != nil else {
            return nil
        }
        let directory = [UInt8](file.readData(ofLength: directoryLength))
        guard directory.count == directoryLength else {
            return nil
        }
        // 32 bytes a section: its kind, an id, where it is and how long
        var i = 0
        while i + 32 <= directory.count {
            if directory[i..<(i + 4)] == [0x54, 0x48, 0x4d, 0x42] {
                let offset = little(directory, i + 8, 8), length = Int(little(directory, i + 16, 8))
                guard length <= 1 << 24, (try? file.seek(toOffset: offset)) != nil else {
                    return nil
                }
                let picture = file.readData(ofLength: length)
                return (picture.count == length) ? picture : nil
            }
            i += 32
        }
        return nil
    }

    /// A little-endian number of `count` bytes at `at`.
    private func little(_ bytes: [UInt8], _ at: Int, _ count: Int) -> UInt64 {
        var value: UInt64 = 0
        for k in (0..<count).reversed() {
            value = value << 8 | UInt64(bytes[at + k])
        }
        return value
    }
}
