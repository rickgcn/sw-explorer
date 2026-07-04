#include "../file_table_model.h"

#include <QApplication>
#include <QByteArray>
#include <QModelIndex>

#include <iostream>
#include <utility>

namespace {

bool check(bool condition, const char *message) {
    if (!condition) {
        std::cerr << message << '\n';
        return false;
    }
    return true;
}

swcore::FileEntry entry(QString fname, QString machExpr) {
    swcore::FileEntry out;
    out.ftype = 'f';
    out.fname = std::move(fname);
    out.subgroup = "demo.sw.base";
    out.subproductBase = "demo.sw";
    out.machExpr = std::move(machExpr);
    return out;
}

int countEntry(const QVector<swcore::FileEntry> &entries, const QString &fname, const QString &machExpr) {
    int count = 0;
    for (const swcore::FileEntry &e : entries) {
        if (e.fname == fname && e.machExpr == machExpr) {
            ++count;
        }
    }
    return count;
}

bool testMachTargetKeepsCommonAndPrefersSpecific() {
    FileTableModel model;
    QVector<swcore::FileEntry> entries;
    entries.push_back(entry("usr/lib/libsame.so", ""));
    entries.push_back(entry("usr/lib/libsame.so", "MODE=32bit"));
    entries.push_back(entry("usr/lib/libsame.so", "MODE=64bit"));
    entries.push_back(entry("usr/lib/libcommon.so", ""));
    entries.push_back(entry("usr/lib/lib32only.so", "MODE=32bit"));

    model.setEntries(entries);
    if (!check(model.totalFilteredEntryCount() == 5, "default filtering should keep all entries")) {
        return false;
    }

    model.setMachFilter("MODE=64bit");
    const QVector<swcore::FileEntry> filtered = model.entriesInCurrentTree();

    if (!check(filtered.size() == 2, "mach filtering should keep one specific file and one common file")) {
        return false;
    }
    if (!check(countEntry(filtered, "usr/lib/libsame.so", "MODE=64bit") == 1,
               "mach-specific entry was not preferred for a duplicate path")) {
        return false;
    }
    if (!check(countEntry(filtered, "usr/lib/libsame.so", "") == 0,
               "common duplicate path should be hidden when a mach-specific entry matches")) {
        return false;
    }
    if (!check(countEntry(filtered, "usr/lib/libcommon.so", "") == 1,
               "common non-duplicate file should be kept")) {
        return false;
    }
    if (!check(countEntry(filtered, "usr/lib/lib32only.so", "MODE=32bit") == 0,
               "non-matching mach-specific file should be excluded")) {
        return false;
    }

    model.setMachFilter("64bit");
    const QVector<swcore::FileEntry> containsFiltered = model.entriesInCurrentTree();
    if (!check(containsFiltered.size() == 2, "bare mach text should match mach expressions by containment")) {
        return false;
    }
    if (!check(countEntry(containsFiltered, "usr/lib/libsame.so", "MODE=64bit") == 1,
               "bare mach text did not prefer the matching mach-specific duplicate")) {
        return false;
    }

    model.setCurrentDirectory("usr/lib");
    if (!check(model.rowCount() == 3, "mach-filtered directory should show parent row and two file rows")) {
        return false;
    }

    QModelIndexList selectedRows;
    for (int row = 0; row < model.rowCount(); ++row) {
        if (model.data(model.index(row, 0)).toString() == "libsame.so") {
            selectedRows.push_back(model.index(row, 0));
        }
    }
    const QVector<swcore::FileEntry> selected = model.entriesForRows(selectedRows);
    if (!check(selected.size() == 1, "selecting duplicate path should return one mach-filtered entry")) {
        return false;
    }
    if (!check(selected.at(0).machExpr == "MODE=64bit", "selected duplicate path did not resolve to target mach")) {
        return false;
    }

    return true;
}

bool testMultipleMachTargetsReportConflicts() {
    FileTableModel model;
    QVector<swcore::FileEntry> entries;
    entries.push_back(entry("usr/lib/libsame.so", ""));
    entries.push_back(entry("usr/lib/libsame.so", "MODE=32bit"));
    entries.push_back(entry("usr/lib/libsame.so", "MODE=64bit"));
    entries.push_back(entry("usr/lib/libcommon.so", ""));
    entries.push_back(entry("usr/lib/lib32only.so", "MODE=32bit"));

    model.setEntries(entries);
    model.setMachFilter(" , MODE=32bit, , MODE=64bit , ");
    const QVector<swcore::FileEntry> filtered = model.entriesInCurrentTree();

    if (!check(filtered.size() == 4, "multiple mach targets should keep matching specific entries and common fallbacks")) {
        return false;
    }
    if (!check(countEntry(filtered, "usr/lib/libsame.so", "MODE=32bit") == 1,
               "multiple mach targets should keep the 32-bit duplicate")) {
        return false;
    }
    if (!check(countEntry(filtered, "usr/lib/libsame.so", "MODE=64bit") == 1,
               "multiple mach targets should keep the 64-bit duplicate")) {
        return false;
    }
    if (!check(countEntry(filtered, "usr/lib/libsame.so", "") == 0,
               "common duplicate path should be hidden when multiple mach-specific entries match")) {
        return false;
    }
    if (!check(countEntry(filtered, "usr/lib/libcommon.so", "") == 1,
               "common non-duplicate file should remain with multiple mach targets")) {
        return false;
    }
    if (!check(countEntry(filtered, "usr/lib/lib32only.so", "MODE=32bit") == 1,
               "single matching mach-specific file should remain with multiple mach targets")) {
        return false;
    }

    const QStringList conflicts = FileTableModel::machConflictSummaries(filtered);
    if (!check(conflicts.size() == 1, "multiple mach-specific entries for one path should report one conflict")) {
        return false;
    }
    if (!check(conflicts.at(0).contains("/usr/lib/libsame.so") &&
                   conflicts.at(0).contains("MODE=32bit") &&
                   conflicts.at(0).contains("MODE=64bit"),
               "conflict summary should include path and both mach values")) {
        return false;
    }

    QVector<swcore::FileEntry> selectedOne;
    selectedOne.push_back(entry("usr/lib/libsame.so", "MODE=64bit"));
    if (!check(FileTableModel::machConflictSummaries(selectedOne).isEmpty(),
               "a single selected mach-specific entry should not report a conflict")) {
        return false;
    }

    return true;
}

bool testMachConditionSpacingDoesNotNeedExactInput() {
    FileTableModel model;
    QVector<swcore::FileEntry> entries;
    entries.push_back(entry("usr/gfx/light", "CPUBOARD=IP12 GFXBOARD=LIGHT GFXBOARD=EXPRESS"));
    entries.push_back(entry("usr/gfx/eclipse", "CPUBOARD=IP12 GFXBOARD=ECLIPSE GFXBOARD=SERVER"));
    entries.push_back(entry("usr/gfx/base", "CPUBOARD=IP12"));

    model.setEntries(entries);

    model.setMachFilter("CPUBOARD=IP12 GFXBOARD=LIGHT");
    QVector<swcore::FileEntry> filtered = model.entriesInCurrentTree();
    if (!check(filtered.size() == 1, "space-separated mach terms should require all terms, not exact phrase matching")) {
        return false;
    }
    if (!check(countEntry(filtered, "usr/gfx/light", "CPUBOARD=IP12 GFXBOARD=LIGHT GFXBOARD=EXPRESS") == 1,
               "space-separated mach terms should match the light board entry")) {
        return false;
    }

    model.setMachFilter("CPUBOARD=IP12GFXBOARD=LIGHT");
    filtered = model.entriesInCurrentTree();
    if (!check(filtered.size() == 1, "plain mach matching should ignore whitespace inside expressions")) {
        return false;
    }
    if (!check(countEntry(filtered, "usr/gfx/light", "CPUBOARD=IP12 GFXBOARD=LIGHT GFXBOARD=EXPRESS") == 1,
               "compact mach input should match the spaced mach expression")) {
        return false;
    }

    model.setMachFilter("CPUBOARD=IP12 GFXBOARD=SERVER");
    filtered = model.entriesInCurrentTree();
    if (!check(filtered.size() == 1, "mach terms should match regardless of trailing sibling conditions")) {
        return false;
    }
    if (!check(countEntry(filtered, "usr/gfx/eclipse", "CPUBOARD=IP12 GFXBOARD=ECLIPSE GFXBOARD=SERVER") == 1,
               "space-separated mach terms should match the server board entry")) {
        return false;
    }

    return true;
}

bool testMachExcludeTargets() {
    FileTableModel model;
    QVector<swcore::FileEntry> entries;
    entries.push_back(entry("usr/gfx/ip12a", "CPUBOARD=IP12"));
    entries.push_back(entry("usr/gfx/light", "CPUBOARD=IP12 GFXBOARD=LIGHT GFXBOARD=EXPRESS"));
    entries.push_back(entry("usr/gfx/eclipse", "CPUBOARD=IP12 GFXBOARD=ECLIPSE GFXBOARD=SERVER"));
    entries.push_back(entry("usr/gfx/ip12b", "CPUBOARD=IP12"));

    model.setEntries(entries);

    model.setMachFilter("CPUBOARD=IP12,!GFXBOARD=ECLIPSE");
    QVector<swcore::FileEntry> filtered = model.entriesInCurrentTree();
    if (!check(filtered.size() == 3, "exclude target should remove only the matching mach expression")) {
        return false;
    }
    if (!check(countEntry(filtered, "usr/gfx/ip12a", "CPUBOARD=IP12") == 1 &&
                   countEntry(filtered, "usr/gfx/light", "CPUBOARD=IP12 GFXBOARD=LIGHT GFXBOARD=EXPRESS") == 1 &&
                   countEntry(filtered, "usr/gfx/ip12b", "CPUBOARD=IP12") == 1,
               "exclude target should keep the non-excluded IP12 entries")) {
        return false;
    }
    if (!check(countEntry(filtered, "usr/gfx/eclipse", "CPUBOARD=IP12 GFXBOARD=ECLIPSE GFXBOARD=SERVER") == 0,
               "exclude target should remove the ECLIPSE/SERVER entry")) {
        return false;
    }

    model.setMachFilter("!GFXBOARD=ECLIPSE");
    filtered = model.entriesInCurrentTree();
    if (!check(filtered.size() == 3, "exclude-only filter should include all non-excluded mach entries")) {
        return false;
    }
    if (!check(countEntry(filtered, "usr/gfx/eclipse", "CPUBOARD=IP12 GFXBOARD=ECLIPSE GFXBOARD=SERVER") == 0,
               "exclude-only filter should remove the excluded mach entry")) {
        return false;
    }

    return true;
}

} // namespace

int main(int argc, char **argv) {
    qputenv("QT_QPA_PLATFORM", QByteArray("offscreen"));
    QApplication app(argc, argv);

    if (!testMachTargetKeepsCommonAndPrefersSpecific()) {
        return 1;
    }
    if (!testMultipleMachTargetsReportConflicts()) {
        return 1;
    }
    if (!testMachConditionSpacingDoesNotNeedExactInput()) {
        return 1;
    }
    if (!testMachExcludeTargets()) {
        return 1;
    }
    return 0;
}
