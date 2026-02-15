#pragma once

#include <QString>
#include <QStringList>
#include <QVector>

namespace swcore {

struct FileEntry {
    QChar ftype;
    int mode = 0;
    QString user;
    QString group;
    QString fname;
    QString sourcePath;
    QString subgroup;
    QString subproductBase;
    QString attrsRaw;
    QString machExpr;
    QString symval;
    qint64 size = 0;
    qint64 cmpsize = 0;
    qint64 payloadSize = 0;
    qint64 offset = -1;
};

struct ParseResult {
    QString product;
    QVector<FileEntry> entries;
    QStringList warnings;
};

struct ExtractOptions {
    bool noDecompress = false;
    bool keepZ = false;
    bool continueOnError = true;
    qint64 resyncBack = 1024 * 1024;
    qint64 resyncForward = 16 * 1024 * 1024;
    qint64 resyncChunk = 1024 * 1024;
};

struct ExtractResult {
    int total = 0;
    int extracted = 0;
    int skipped = 0;
    int errors = 0;
    bool canceled = false;
    QStringList errorMessages;
};

} // namespace swcore

