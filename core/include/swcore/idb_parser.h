#pragma once

#include "swcore/types.h"

namespace swcore {

class IdbParser {
public:
    static QStringList findProducts(const QString &distDirPath);
    static ParseResult parse(const QString &distDirPath, const QString &product, QString *errorMessage = nullptr);
};

} // namespace swcore

