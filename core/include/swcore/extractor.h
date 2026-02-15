#pragma once

#include "swcore/types.h"

#include <functional>

namespace swcore {

class DistExtractor {
public:
    using ProgressCallback = std::function<bool(int current, int total, const QString &name)>;

    static ExtractResult extract(const QString &distDirPath,
                                 const QVector<FileEntry> &entries,
                                 const QString &outDirPath,
                                 const ExtractOptions &options,
                                 const ProgressCallback &progress = {});
};

} // namespace swcore

