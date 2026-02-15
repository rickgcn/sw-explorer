#include "swcore/extractor.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QFileDevice>
#include <QPair>
#include <QSaveFile>
#include <QSet>

#include <algorithm>
#include <map>
#include <memory>
#include <optional>
#include <stdexcept>
#include <utility>
#include <vector>

namespace swcore {

namespace {

class CompressCodeReader {
public:
    explicit CompressCodeReader(QByteArray data) : m_data(std::move(data)) {}

    int nextCode(int maxBits, int maxMaxCode, int *nBits, int *maxCode, int freeEnt, bool *clearFlag) {
        if (!nBits || !maxCode || !clearFlag) {
            return -1;
        }

        if (*clearFlag || m_offset >= m_size || freeEnt > *maxCode) {
            if (freeEnt > *maxCode) {
                ++(*nBits);
                if (*nBits == maxBits) {
                    *maxCode = maxMaxCode;
                } else {
                    *maxCode = (1 << *nBits) - 1;
                }
            }

            if (*clearFlag) {
                *nBits = 9;
                *maxCode = (1 << *nBits) - 1;
                *clearFlag = false;
            }

            const int remain = m_data.size() - m_pos;
            if (remain <= 0) {
                return -1;
            }

            const int chunkBytes = std::min(*nBits, remain);
            m_chunk = m_data.mid(m_pos, chunkBytes);
            m_pos += chunkBytes;

            m_offset = 0;
            m_size = (chunkBytes << 3) - (*nBits - 1);
            if (m_size <= 0) {
                return -1;
            }
        }

        if (m_offset >= m_size) {
            return -1;
        }

        const int startBit = m_offset;
        const int endBit = startBit + *nBits - 1;
        const int endByte = endBit >> 3;
        if (endByte >= m_chunk.size()) {
            return -1;
        }

        static constexpr quint32 rmask[9] = {
            0x00, 0x01, 0x03, 0x07, 0x0F, 0x1F, 0x3F, 0x7F, 0xFF
        };

        int bitOffset = startBit & 7;
        int bitsLeft = *nBits;
        const uchar *bp = reinterpret_cast<const uchar *>(m_chunk.constData()) + (startBit >> 3);

        quint32 code = quint32(*bp++ >> bitOffset);
        bitsLeft -= 8 - bitOffset;
        int shift = 8 - bitOffset;

        while (bitsLeft >= 8) {
            code |= quint32(*bp++) << shift;
            shift += 8;
            bitsLeft -= 8;
        }

        if (bitsLeft > 0) {
            code |= (quint32(*bp) & rmask[bitsLeft]) << shift;
        }

        m_offset += *nBits;
        return int(code);
    }

private:
    QByteArray m_data;
    int m_pos = 0;
    QByteArray m_chunk;
    int m_offset = 0;
    int m_size = 0;
};

QByteArray unlzw(const QByteArray &input) {
    if (input.size() < 3 || quint8(input.at(0)) != 0x1F || quint8(input.at(1)) != 0x9D) {
        throw std::runtime_error("Not a .Z stream");
    }

    const int flags = quint8(input.at(2));
    const int maxbits = flags & 0x1F;
    const bool blockMode = (flags & 0x80) != 0;
    if (maxbits < 9 || maxbits > 16) {
        throw std::runtime_error("Unsupported .Z maxbits");
    }

    const int clearCode = 256;
    const int firstCode = 257;
    const int maxMaxCode = 1 << maxbits;

    int nBits = 9;
    int maxCode = (1 << nBits) - 1;
    int freeEnt = blockMode ? firstCode : 256;
    bool clearFlag = false;

    CompressCodeReader reader(input.mid(3));

    std::vector<int> prefix(maxMaxCode, 0);
    std::vector<quint8> suffix(maxMaxCode, 0);
    std::vector<quint8> stack(maxMaxCode, 0);
    for (int i = 0; i < 256; ++i) {
        suffix[i] = quint8(i);
    }

    QByteArray output;

    int oldCode = -1;
    quint8 finChar = 0;

    while (true) {
        int code = reader.nextCode(maxbits, maxMaxCode, &nBits, &maxCode, freeEnt, &clearFlag);
        if (code < 0) {
            break;
        }

        if (blockMode && code == clearCode) {
            clearFlag = true;
            freeEnt = firstCode;
            oldCode = -1;
            continue;
        }

        if (oldCode < 0) {
            if (code > 255) {
                throw std::runtime_error("Corrupt .Z stream");
            }
            finChar = quint8(code);
            output.append(char(finChar));
            oldCode = code;
            continue;
        }

        const int inCode = code;
        int stackTop = 0;

        if (code >= freeEnt) {
            if (code != freeEnt) {
                throw std::runtime_error("LZW decode error");
            }
            if (stackTop >= int(stack.size())) {
                throw std::runtime_error("LZW stack overflow");
            }
            stack[stackTop++] = finChar;
            code = oldCode;
        }

        while (code >= 256) {
            if (code >= freeEnt || code < 0) {
                throw std::runtime_error("LZW decode error");
            }
            if (stackTop >= int(stack.size())) {
                throw std::runtime_error("LZW stack overflow");
            }
            stack[stackTop++] = suffix[code];
            code = prefix[code];
        }

        finChar = quint8(code & 0xFF);
        if (stackTop >= int(stack.size())) {
            throw std::runtime_error("LZW stack overflow");
        }
        stack[stackTop++] = finChar;

        while (stackTop > 0) {
            output.append(char(stack[--stackTop]));
        }

        if (freeEnt < maxMaxCode) {
            prefix[freeEnt] = oldCode;
            suffix[freeEnt] = finChar;
            ++freeEnt;
        }

        oldCode = inCode;
    }

    return output;
}

QString sanitizeRelativePath(QString p) {
    while (p.startsWith('/')) {
        p.remove(0, 1);
    }
    const QStringList segs = p.split('/', Qt::SkipEmptyParts);
    QStringList clean;
    clean.reserve(segs.size());
    for (const QString &s : segs) {
        if (s == "." || s == "..") {
            continue;
        }
        clean.push_back(s);
    }
    return clean.join('/');
}

bool ensureParentDir(const QString &path) {
    return QDir().mkpath(QFileInfo(path).path());
}

QFileDevice::Permissions modeToPermissions(int mode) {
    QFileDevice::Permissions p;
    if (mode & 0400) p |= QFileDevice::ReadOwner;
    if (mode & 0200) p |= QFileDevice::WriteOwner;
    if (mode & 0100) p |= QFileDevice::ExeOwner;
    if (mode & 0040) p |= QFileDevice::ReadGroup;
    if (mode & 0020) p |= QFileDevice::WriteGroup;
    if (mode & 0010) p |= QFileDevice::ExeGroup;
    if (mode & 0004) p |= QFileDevice::ReadOther;
    if (mode & 0002) p |= QFileDevice::WriteOther;
    if (mode & 0001) p |= QFileDevice::ExeOther;
    return p;
}

QList<QByteArray> nameVariants(const QString &name) {
    const QByteArray raw = name.toLatin1();
    QList<QByteArray> variants;
    variants << raw << ("./" + raw) << ("/" + raw);

    QList<QByteArray> unique;
    QSet<QByteArray> seen;
    for (const QByteArray &v : variants) {
        if (!seen.contains(v)) {
            unique.push_back(v);
            seen.insert(v);
        }
    }
    return unique;
}

bool checkHeaderAt(QFile &file, qint64 offset, const QByteArray &nameBytes) {
    if (offset < 0 || !file.seek(offset)) {
        return false;
    }
    const QByteArray hdr = file.read(nameBytes.size() + 2);
    if (hdr.size() != nameBytes.size() + 2) {
        return false;
    }
    const quint16 declaredLen = (quint16(quint8(hdr.at(0))) << 8) | quint16(quint8(hdr.at(1)));
    return declaredLen == quint16(nameBytes.size()) && hdr.mid(2) == nameBytes;
}

std::optional<QPair<qint64, QByteArray>> resyncOffset(QFile &file,
                                                       const QList<QByteArray> &variants,
                                                       qint64 baseOffset,
                                                       qint64 back,
                                                       qint64 forward,
                                                       qint64 chunkSize) {
    if (variants.isEmpty()) {
        return std::nullopt;
    }

    const qint64 scanStart = std::max<qint64>(0, baseOffset - back);
    const qint64 scanEnd = std::min(file.size(), baseOffset + forward);
    if (scanStart >= scanEnd) {
        return std::nullopt;
    }

    int maxNameLen = 0;
    for (const QByteArray &v : variants) {
        maxNameLen = std::max(maxNameLen, int(v.size()));
    }
    const qint64 overlap = maxNameLen + 2;
    qint64 pos = scanStart;

    while (pos < scanEnd) {
        const qint64 toRead = std::min(chunkSize, scanEnd - pos);
        if (!file.seek(pos)) {
            return std::nullopt;
        }
        const QByteArray blob = file.read(toRead);
        if (blob.isEmpty()) {
            break;
        }

        for (const QByteArray &name : variants) {
            int found = blob.indexOf(name);
            while (found >= 0) {
                if (found >= 2) {
                    const qint64 candidate = pos + found - 2;
                    if (checkHeaderAt(file, candidate, name)) {
                        return QPair<qint64, QByteArray>(candidate, name);
                    }
                }
                found = blob.indexOf(name, found + 1);
            }
        }

        if (toRead <= overlap) {
            break;
        }
        pos += toRead - overlap;
    }

    return std::nullopt;
}

struct SubRuntime {
    explicit SubRuntime(QString path) : filePath(std::move(path)), file(filePath) {}

    QString filePath;
    QFile file;
    qint64 delta = 0;
};

SubRuntime *ensureSubRuntime(const QString &distDirPath,
                             const QString &subBase,
                             std::map<QString, std::unique_ptr<SubRuntime>> *subs,
                             QString *error) {
    if (!subs) {
        return nullptr;
    }
    auto it = subs->find(subBase);
    if (it != subs->end()) {
        return it->second.get();
    }

    const QString subPath = QDir(distDirPath).filePath(subBase);
    auto runtime = std::make_unique<SubRuntime>(subPath);
    if (!runtime->file.open(QIODevice::ReadOnly)) {
        if (error) {
            *error = QString("Cannot open subproduct file: %1").arg(subPath);
        }
        return nullptr;
    }

    const auto inserted = subs->emplace(subBase, std::move(runtime));
    return inserted.first->second.get();
}

bool readPayload(SubRuntime *sub,
                 const FileEntry &entry,
                 const ExtractOptions &options,
                 QByteArray *payload,
                 QString *error) {
    if (!sub || !payload) {
        if (error) {
            *error = "Internal error reading payload";
        }
        return false;
    }
    if (entry.payloadSize < 0 || entry.offset < 0) {
        if (error) {
            *error = "Invalid payload metadata";
        }
        return false;
    }

    QFile &file = sub->file;
    const QList<QByteArray> variants = nameVariants(entry.fname);
    qint64 wantOff = entry.offset + sub->delta;
    QByteArray matched;

    for (const QByteArray &name : variants) {
        if (checkHeaderAt(file, wantOff, name)) {
            matched = name;
            break;
        }
    }

    if (matched.isEmpty()) {
        const auto res = resyncOffset(file,
                                      variants,
                                      wantOff,
                                      options.resyncBack,
                                      options.resyncForward,
                                      std::max<qint64>(4096, options.resyncChunk));
        if (!res.has_value() && sub->delta != 0) {
            for (const QByteArray &name : variants) {
                if (checkHeaderAt(file, entry.offset, name)) {
                    sub->delta = 0;
                    wantOff = entry.offset;
                    matched = name;
                    break;
                }
            }
        }

        if (matched.isEmpty()) {
            if (!res.has_value()) {
                if (error) {
                    *error = QString("Out of sync at %1 (delta=%2)")
                                 .arg(entry.offset)
                                 .arg(sub->delta);
                }
                return false;
            }

            const qint64 candidate = res->first;
            matched = res->second;
            const qint64 newDelta = candidate - entry.offset;
            sub->delta = newDelta;
            wantOff = candidate;
        }
    }

    if (!file.seek(wantOff + 2 + matched.size())) {
        if (error) {
            *error = QString("Seek failed at %1").arg(wantOff);
        }
        return false;
    }

    const QByteArray data = file.read(entry.payloadSize);
    if (data.size() != entry.payloadSize) {
        if (error) {
            *error = QString("Short read for %1").arg(entry.fname);
        }
        return false;
    }
    *payload = data;
    return true;
}

bool writeBytes(const QString &path, const QByteArray &bytes, int mode, QString *error, bool applyMode = true) {
    if (!ensureParentDir(path)) {
        if (error) {
            *error = QString("Cannot create parent directory for %1").arg(path);
        }
        return false;
    }

    QFileInfo fi(path);
    if (fi.exists()) {
        if (fi.isDir()) {
            if (error) {
                *error = QString("Output path is a directory: %1").arg(path);
            }
            return false;
        }
        QFile existing(path);
        QFileDevice::Permissions perms = existing.permissions();
        perms |= QFileDevice::WriteOwner;
        existing.setPermissions(perms);
    }

    QSaveFile out(path);
    if (!out.open(QIODevice::WriteOnly)) {
        if (error) {
            *error = QString("Cannot open output file %1").arg(path);
        }
        return false;
    }
    if (out.write(bytes) != bytes.size()) {
        if (error) {
            *error = QString("Write failed for %1").arg(path);
        }
        return false;
    }
    if (!out.commit()) {
        if (error) {
            *error = QString("Commit failed for %1").arg(path);
        }
        return false;
    }
    if (applyMode) {
        QFile(path).setPermissions(modeToPermissions(mode));
    }
    return true;
}

bool removeFileEvenIfReadonly(const QString &path) {
    QFile f(path);
    if (!f.exists()) {
        return true;
    }
    QFileDevice::Permissions p = f.permissions();
    p |= QFileDevice::WriteOwner;
    f.setPermissions(p);
    return f.remove();
}

bool writeAndSetMode(const QString &path, const QByteArray &bytes, int mode, QString *error) {
    return writeBytes(path, bytes, mode, error, true);
}

bool writeTempBytes(const QString &path, const QByteArray &bytes, QString *error) {
    // Temporary .Z payload should stay writable so cleanup can remove it on Windows.
    return writeBytes(path, bytes, 0, error, false);
}

bool writeSymlinkFallback(const QString &path, const QString &target, QString *error) {
    return writeAndSetMode(path, target.toUtf8(), 0644, error);
}

bool writeEmptyFile(const QString &path, int mode, QString *error) {
    return writeAndSetMode(path, QByteArray(), mode, error);
}

bool writeExtractedFile(const QString &path, const QByteArray &raw, int mode, QString *error) {
    return writeAndSetMode(path, raw, mode, error);
}

bool writeCompressedTemp(const QString &path, const QByteArray &payload, QString *error) {
    return writeTempBytes(path, payload, error);
}

bool removeCompressedTemp(const QString &path, QString *error) {
    if (removeFileEvenIfReadonly(path)) {
        return true;
    }
    if (error) {
        *error = QString("Cannot remove temp file %1").arg(path);
    }
    return false;
}

bool extractOne(const QString &distDirPath,
                const QString &outDirPath,
                const FileEntry &entry,
                const ExtractOptions &options,
                std::map<QString, std::unique_ptr<SubRuntime>> *subStates,
                QString *error) {
    const QString safeRel = sanitizeRelativePath(entry.fname);
    const QString dstPath = safeRel.isEmpty() ? outDirPath : QDir(outDirPath).filePath(safeRel);

    if (entry.ftype == 'd') {
        if (!QDir().mkpath(dstPath)) {
            if (error) {
                *error = QString("Cannot create directory %1").arg(dstPath);
            }
            return false;
        }
        QFile(dstPath).setPermissions(modeToPermissions(entry.mode));
        return true;
    }

    if (entry.ftype == 'l') {
        if (!ensureParentDir(dstPath)) {
            if (error) {
                *error = QString("Cannot create parent for symlink %1").arg(dstPath);
            }
            return false;
        }
        QFile::remove(dstPath);
        if (!QFile::link(entry.symval, dstPath)) {
            const QString linkMeta = dstPath + ".link.txt";
            return writeSymlinkFallback(linkMeta, entry.symval, error);
        }
        return true;
    }

    if (entry.ftype != 'f') {
        return false;
    }

    if (entry.payloadSize == 0) {
        return writeEmptyFile(dstPath, entry.mode, error);
    }

    QString runtimeError;
    SubRuntime *sub = ensureSubRuntime(distDirPath, entry.subproductBase, subStates, &runtimeError);
    if (!sub) {
        if (error) {
            *error = runtimeError;
        }
        return false;
    }

    QByteArray payload;
    if (!readPayload(sub, entry, options, &payload, &runtimeError)) {
        if (error) {
            *error = runtimeError;
        }
        return false;
    }

    const QString zPath = dstPath + ".Z";
    if (!writeCompressedTemp(zPath, payload, &runtimeError)) {
        if (error) {
            *error = runtimeError;
        }
        return false;
    }

    if (options.noDecompress) {
        return true;
    }

    QByteArray raw = payload;
    if (payload.size() >= 2 && quint8(payload.at(0)) == 0x1F && quint8(payload.at(1)) == 0x9D) {
        try {
            raw = unlzw(payload);
        } catch (const std::exception &) {
            if (error) {
                *error = QString("LZW decompress failed: %1").arg(entry.fname);
            }
            return false;
        }
    }

    if (!writeExtractedFile(dstPath, raw, entry.mode, &runtimeError)) {
        if (error) {
            *error = runtimeError;
        }
        return false;
    }

    if (!options.keepZ) {
        if (!removeCompressedTemp(zPath, &runtimeError)) {
            if (error) {
                *error = runtimeError;
            }
            return false;
        }
    }
    return true;
}

} // namespace

ExtractResult DistExtractor::extract(const QString &distDirPath,
                                     const QVector<FileEntry> &entries,
                                     const QString &outDirPath,
                                     const ExtractOptions &options,
                                     const ProgressCallback &progress) {
    ExtractResult result;
    result.total = entries.size();

    if (!QDir().mkpath(outDirPath)) {
        result.errors = 1;
        result.errorMessages.push_back(QString("Cannot create output directory: %1").arg(outDirPath));
        return result;
    }

    std::map<QString, std::unique_ptr<SubRuntime>> subStates;
    for (int i = 0; i < entries.size(); ++i) {
        const FileEntry &entry = entries.at(i);

        if (progress && !progress(i + 1, entries.size(), entry.fname)) {
            result.canceled = true;
            break;
        }

        if (entry.ftype != 'f' && entry.ftype != 'd' && entry.ftype != 'l') {
            ++result.skipped;
            continue;
        }

        QString error;
        const bool ok = extractOne(distDirPath, outDirPath, entry, options, &subStates, &error);
        if (ok) {
            ++result.extracted;
            continue;
        }

        ++result.errors;
        result.errorMessages.push_back(QString("%1: %2").arg(entry.fname, error));
        if (!options.continueOnError) {
            break;
        }
    }

    return result;
}

} // namespace swcore
