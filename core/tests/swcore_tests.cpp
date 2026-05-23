#include "swcore/extractor.h"
#include "swcore/idb_parser.h"

#include <QByteArray>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QTemporaryDir>

#include <iostream>

namespace {

bool check(bool condition, const char *message) {
    if (!condition) {
        std::cerr << message << '\n';
        return false;
    }
    return true;
}

bool writeFile(const QString &path, const QByteArray &bytes) {
    QDir().mkpath(QFileInfo(path).path());
    QFile f(path);
    if (!f.open(QIODevice::WriteOnly | QIODevice::Truncate)) {
        return false;
    }
    return f.write(bytes) == bytes.size();
}

QByteArray distHeader() {
    QByteArray out("im001V620P02", 12);
    out.append('\0');
    return out;
}

QByteArray payloadRecord(const QString &name, const QByteArray &payload) {
    const QByteArray nameBytes = name.toLatin1();
    QByteArray out;
    out.append(char((nameBytes.size() >> 8) & 0xFF));
    out.append(char(nameBytes.size() & 0xFF));
    out.append(nameBytes);
    out.append(payload);
    return out;
}

bool testSw64SubproductExtraction() {
    QTemporaryDir temp;
    if (!check(temp.isValid(), "temporary directory creation failed")) {
        return false;
    }

    const QString distDir = QDir(temp.path()).filePath("dist");
    if (!check(QDir().mkpath(distDir), "dist directory creation failed")) {
        return false;
    }

    const QString fileName = "usr/lib64/libdemo.so";
    const QByteArray filePayload("SW64DATA");
    const QString subproduct = "demo.extra.sw64";
    const QByteArray subproductBytes = distHeader() + payloadRecord(fileName, filePayload);

    if (!check(writeFile(QDir(distDir).filePath(subproduct), subproductBytes), "subproduct write failed")) {
        return false;
    }

    const QString idbLine =
        QString("f 0644 root sys %1 source/libdemo.so demo.extra.sw64.libs "
                "mach(MODE=64bit) size(%2) cmpsize(%2)\n")
            .arg(fileName)
            .arg(filePayload.size());
    if (!check(writeFile(QDir(distDir).filePath("demo.idb"), idbLine.toLatin1()), "idb write failed")) {
        return false;
    }

    QString error;
    const swcore::ParseResult parsed = swcore::IdbParser::parse(distDir, "demo", &error);
    if (!check(error.isEmpty(), "parser returned an error")) {
        std::cerr << error.toStdString() << '\n';
        return false;
    }
    if (!check(parsed.entries.size() == 1, "parser did not return the sw64 entry")) {
        return false;
    }

    const swcore::FileEntry &entry = parsed.entries.at(0);
    if (!check(entry.subproductBase == subproduct, "parser did not keep the full sw64 subproduct base")) {
        std::cerr << entry.subproductBase.toStdString() << '\n';
        return false;
    }
    if (!check(entry.offset == 13, "parser computed an unexpected payload offset")) {
        return false;
    }
    if (!check(entry.machExpr == "MODE=64bit", "parser did not preserve the mach expression")) {
        return false;
    }

    swcore::ExtractOptions options;
    const QString outDir = QDir(temp.path()).filePath("out");
    const swcore::ExtractResult extracted = swcore::DistExtractor::extract(distDir, parsed.entries, outDir, options);
    if (!check(extracted.errors == 0, "sw64 extraction returned errors")) {
        for (const QString &msg : extracted.errorMessages) {
            std::cerr << msg.toStdString() << '\n';
        }
        return false;
    }
    if (!check(extracted.extracted == 1, "sw64 extraction count was unexpected")) {
        return false;
    }

    QFile out(QDir(outDir).filePath("libdemo.so"));
    if (!check(out.open(QIODevice::ReadOnly), "default flat sw64 file was not written")) {
        return false;
    }
    if (!check(out.readAll() == filePayload, "default flat sw64 file content did not match")) {
        return false;
    }
    if (!check(!QFileInfo::exists(QDir(outDir).filePath(fileName)), "default extraction unexpectedly preserved paths")) {
        return false;
    }
    if (!check(!QFileInfo::exists(QDir(outDir).filePath("libdemo.so.Z")), "temporary .Z file was not removed")) {
        return false;
    }

    options.keepRelativePaths = true;
    options.relativePathRoot.clear();
    const QString relativeOutDir = QDir(temp.path()).filePath("out-relative");
    const swcore::ExtractResult extractedRelative =
        swcore::DistExtractor::extract(distDir, parsed.entries, relativeOutDir, options);
    if (!check(extractedRelative.errors == 0, "relative-path sw64 extraction returned errors")) {
        for (const QString &msg : extractedRelative.errorMessages) {
            std::cerr << msg.toStdString() << '\n';
        }
        return false;
    }
    QFile relativeOut(QDir(relativeOutDir).filePath(fileName));
    if (!check(relativeOut.open(QIODevice::ReadOnly), "relative-path sw64 file was not written")) {
        return false;
    }
    if (!check(relativeOut.readAll() == filePayload, "relative-path sw64 file content did not match")) {
        return false;
    }

    options.preservePaths = true;
    options.keepRelativePaths = false;
    const QString fullOutDir = QDir(temp.path()).filePath("out-full");
    const swcore::ExtractResult extractedFull = swcore::DistExtractor::extract(distDir, parsed.entries, fullOutDir, options);
    if (!check(extractedFull.errors == 0, "preserve-path sw64 extraction returned errors")) {
        for (const QString &msg : extractedFull.errorMessages) {
            std::cerr << msg.toStdString() << '\n';
        }
        return false;
    }
    QFile fullOut(QDir(fullOutDir).filePath(fileName));
    if (!check(fullOut.open(QIODevice::ReadOnly), "preserve-path sw64 file was not written")) {
        return false;
    }
    if (!check(fullOut.readAll() == filePayload, "preserve-path sw64 file content did not match")) {
        return false;
    }

    return true;
}

} // namespace

int main() {
    if (!testSw64SubproductExtraction()) {
        return 1;
    }
    return 0;
}
