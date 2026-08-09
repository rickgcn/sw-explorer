#pragma once

#include <QString>
#include <QtMath>

// Deterministic IEC byte formatting shared by the entry table and the
// inspector: 812 bytes -> "812 B", 832,512 -> "812 KiB",
// 1,363,148 -> "1.3 MiB". Locale-independent, so table cells and
// detail rows read identically on every machine.
inline QString formatByteSize(quint64 bytes)
{
    if (bytes < 1024) {
        return QStringLiteral("%1 B").arg(bytes);
    }
    static const char *const units[] = {"KiB", "MiB", "GiB", "TiB"};
    double value = static_cast<double>(bytes);
    int unit = -1;
    do {
        value /= 1024.0;
        ++unit;
    } while (value >= 1024.0 && unit < 3);
    // Whole values read without decimals ("812 KiB"), fractional
    // ones keep a single digit ("1.3 MiB").
    const double rounded = qRound64(value * 10.0) / 10.0;
    if (rounded == qFloor(rounded)) {
        return QStringLiteral("%1 %2")
            .arg(static_cast<qint64>(rounded))
            .arg(QLatin1String(units[unit]));
    }
    return QStringLiteral("%1 %2").arg(rounded, 0, 'f', 1).arg(QLatin1String(units[unit]));
}
