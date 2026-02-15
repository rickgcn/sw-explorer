#include "swcore/idb_parser.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QMap>
#include <QRegularExpression>
#include <QTextStream>
#include <QtGlobal>
#if QT_VERSION >= QT_VERSION_CHECK(6, 0, 0)
#include <QStringConverter>
#endif

namespace swcore {

namespace {

constexpr qint64 kIdbHeaderLength = 13;

struct AttrInfo {
    qint64 size = 0;
    qint64 cmpsize = 0;
    QString symval;
    QString machExpr;
};

QString subproductBase(const QString &subgroup) {
    const QStringList parts = subgroup.split('.', Qt::SkipEmptyParts);
    if (parts.size() < 2) {
        return {};
    }
    return parts.at(0) + "." + parts.at(1);
}

QStringList splitMax(const QString &line, int maxParts) {
    QStringList out;
    out.reserve(maxParts);
    const int n = line.size();
    int i = 0;

    while (i < n && out.size() < maxParts - 1) {
        while (i < n && line.at(i).isSpace()) {
            ++i;
        }
        if (i >= n) {
            break;
        }
        const int start = i;
        while (i < n && !line.at(i).isSpace()) {
            ++i;
        }
        out.push_back(line.mid(start, i - start));
    }

    while (i < n && line.at(i).isSpace()) {
        ++i;
    }
    if (i < n && out.size() < maxParts) {
        out.push_back(line.mid(i));
    }
    return out;
}

bool looksLikeSubgroupToken(const QString &token) {
    if (token.contains('(') || token.contains(')') || token.contains('/') || token.contains('=')) {
        return false;
    }
    static const QRegularExpression re(
        "^[A-Za-z0-9_+.-]+\\.[A-Za-z0-9_+.-]+\\.[A-Za-z0-9_+.-]+(?:\\.[A-Za-z0-9_+.-]+)*$");
    return re.match(token).hasMatch();
}

int findSubgroupTokenIndex(const QStringList &tailTokens, const QString &distDirPath) {
    // Prefer a token whose subproduct base exists as a file in dist dir.
    for (int i = 0; i < tailTokens.size(); ++i) {
        const QString tok = tailTokens.at(i);
        const QString base = subproductBase(tok);
        if (base.isEmpty()) {
            continue;
        }
        const QString subPath = QDir(distDirPath).filePath(base);
        if (QFileInfo::exists(subPath)) {
            return i;
        }
    }

    // Fallback for unusual dist layouts.
    for (int i = 0; i < tailTokens.size(); ++i) {
        if (looksLikeSubgroupToken(tailTokens.at(i))) {
            return i;
        }
    }
    return -1;
}

void parseAttrs(const QString &attrs, AttrInfo *info) {
    if (!info) {
        return;
    }

    int i = 0;
    const int n = attrs.size();
    while (i < n) {
        while (i < n && attrs.at(i).isSpace()) {
            ++i;
        }
        if (i >= n) {
            break;
        }

        const int keyStart = i;
        while (i < n && !attrs.at(i).isSpace() && attrs.at(i) != '(') {
            ++i;
        }
        const QString key = attrs.mid(keyStart, i - keyStart).trimmed();
        if (key.isEmpty()) {
            ++i;
            continue;
        }

        if (i >= n || attrs.at(i) != '(') {
            continue;
        }

        ++i;
        const int valStart = i;
        QString value;
        while (true) {
            const int close = attrs.indexOf(')', i);
            if (close < 0) {
                value = attrs.mid(valStart).trimmed();
                i = n;
                break;
            }
            if (close > 0 && attrs.at(close - 1) == ':') {
                i = close + 1;
                continue;
            }
            value = attrs.mid(valStart, close - valStart);
            i = close + 1;
            break;
        }

        const QString lower = key.toLower();
        if (lower == "cmpsize") {
            info->cmpsize = value.trimmed().toLongLong();
        } else if (lower == "size") {
            info->size = value.trimmed().toLongLong();
        } else if (lower == "symval") {
            info->symval = value;
        } else if (lower == "mach") {
            info->machExpr = value.trimmed();
        }
    }
}

} // namespace

QStringList IdbParser::findProducts(const QString &distDirPath) {
    QDir dir(distDirPath);
    QStringList products;
    const QFileInfoList files = dir.entryInfoList({"*.idb"}, QDir::Files, QDir::Name);
    for (const QFileInfo &fi : files) {
        products.push_back(fi.completeBaseName());
    }
    return products;
}

ParseResult IdbParser::parse(const QString &distDirPath, const QString &product, QString *errorMessage) {
    ParseResult result;
    result.product = product;

    auto setError = [&](const QString &msg) {
        if (errorMessage) {
            *errorMessage = msg;
        }
    };

    const QString idbPath = QDir(distDirPath).filePath(product + ".idb");
    QFile idbFile(idbPath);
    if (!idbFile.open(QIODevice::ReadOnly)) {
        setError(QString("Cannot open idb: %1").arg(idbPath));
        return {};
    }

    QTextStream ts(&idbFile);
#if QT_VERSION >= QT_VERSION_CHECK(6, 0, 0)
    ts.setEncoding(QStringConverter::Latin1);
#else
    ts.setCodec("ISO-8859-1");
#endif

    QMap<QString, qint64> curoffBySub;
    int lineNo = 0;
    while (!ts.atEnd()) {
        const QString line = ts.readLine();
        ++lineNo;
        if (line.trimmed().isEmpty()) {
            continue;
        }

        const QStringList tokens = line.split(QRegularExpression("\\s+"), Qt::SkipEmptyParts);
        if (tokens.size() < 7) {
            result.warnings.push_back(QString("Line %1 ignored: not enough fields").arg(lineNo));
            continue;
        }

        FileEntry entry;
        entry.ftype = tokens.at(0).isEmpty() ? QChar() : tokens.at(0).at(0);
        bool modeOk = false;
        entry.mode = tokens.at(1).toInt(&modeOk, 8);
        if (!modeOk) {
            entry.mode = 0;
        }
        entry.user = tokens.at(2);
        entry.group = tokens.at(3);
        entry.fname = tokens.at(4);
        entry.sourcePath = tokens.at(5);

        QStringList tailTokens = tokens.mid(6);
        const int subgroupIdx = findSubgroupTokenIndex(tailTokens, distDirPath);
        if (subgroupIdx < 0) {
            result.warnings.push_back(QString("Line %1 ignored: cannot locate subgroup token").arg(lineNo));
            continue;
        }
        entry.subgroup = tailTokens.at(subgroupIdx);
        tailTokens.removeAt(subgroupIdx);
        entry.attrsRaw = tailTokens.join(' ');
        entry.subproductBase = subproductBase(entry.subgroup);

        if (entry.subproductBase.isEmpty()) {
            result.warnings.push_back(QString("Line %1 ignored: bad subgroup '%2'").arg(lineNo).arg(entry.subgroup));
            continue;
        }

        if (!curoffBySub.contains(entry.subproductBase)) {
            const QString subPath = QDir(distDirPath).filePath(entry.subproductBase);
            if (!QFileInfo::exists(subPath)) {
                setError(QString("Missing subproduct file '%1' referenced by %2:%3")
                             .arg(entry.subproductBase, QFileInfo(idbPath).fileName())
                             .arg(lineNo));
                return {};
            }
            curoffBySub.insert(entry.subproductBase, kIdbHeaderLength);
        }

        if (!entry.attrsRaw.isEmpty()) {
            AttrInfo info;
            parseAttrs(entry.attrsRaw, &info);
            entry.size = info.size;
            entry.cmpsize = info.cmpsize;
            entry.symval = info.symval;
            entry.machExpr = info.machExpr;
        }

        if (entry.ftype == 'f') {
            const qint64 payload = entry.cmpsize > 0 ? entry.cmpsize : entry.size;
            entry.payloadSize = payload;
            entry.offset = curoffBySub.value(entry.subproductBase);

            const qint64 nameLen = entry.fname.toLatin1().size();
            curoffBySub[entry.subproductBase] = entry.offset + payload + nameLen + 2;
        }

        result.entries.push_back(entry);
    }

    if (errorMessage) {
        errorMessage->clear();
    }
    return result;
}

} // namespace swcore
