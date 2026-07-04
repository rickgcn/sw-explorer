#include "file_table_model.h"

#include <QApplication>
#include <QList>
#include <QMap>
#include <QSet>
#include <QStyle>

#include <algorithm>

namespace {

QString joinPath(const QString &parent, const QString &name) {
    if (parent.isEmpty()) {
        return name;
    }
    return parent + "/" + name;
}

} // namespace

FileTableModel::FileTableModel(QObject *parent) : QAbstractTableModel(parent) {
    if (qApp) {
        m_upIcon = qApp->style()->standardIcon(QStyle::SP_FileDialogToParent);
        m_dirIcon = qApp->style()->standardIcon(QStyle::SP_DirIcon);
        m_fileIcon = qApp->style()->standardIcon(QStyle::SP_FileIcon);
        m_linkIcon = qApp->style()->standardIcon(QStyle::SP_FileLinkIcon);
    }
}

int FileTableModel::rowCount(const QModelIndex &parent) const {
    if (parent.isValid()) {
        return 0;
    }
    return m_rows.size();
}

int FileTableModel::columnCount(const QModelIndex &parent) const {
    if (parent.isValid()) {
        return 0;
    }
    return 7;
}

QVariant FileTableModel::data(const QModelIndex &index, int role) const {
    if (!index.isValid() || index.row() < 0 || index.row() >= m_rows.size()) {
        return {};
    }

    const RowItem &row = m_rows.at(index.row());

    if (role == Qt::DecorationRole && index.column() == 0) {
        if (row.kind == RowKind::Parent) {
            return m_upIcon;
        }
        if (row.kind == RowKind::Directory) {
            return m_dirIcon;
        }
        if (row.kind == RowKind::DirectoryLink || row.ftype == 'l') {
            return m_linkIcon.isNull() ? m_fileIcon : m_linkIcon;
        }
        return m_fileIcon;
    }

    if (role == Qt::ToolTipRole) {
        if (row.ftype == 'l') {
            QString tip = QString("%1 -> %2").arg(row.name, row.linkTarget);
            if (row.kind == RowKind::DirectoryLink) {
                tip += "\nDouble-click to enter target directory";
            }
            return tip;
        }
        return {};
    }

    if (role != Qt::DisplayRole) {
        return {};
    }

    switch (index.column()) {
    case 0:
        if (row.kind == RowKind::Parent) {
            return "..";
        }
        if (row.ftype == 'l') {
            return QString("%1 -> %2").arg(row.name, row.linkTarget);
        }
        return row.name;
    case 1:
        return row.kind == RowKind::Entry ? QVariant(row.size) : QVariant();
    case 2:
        return row.kind == RowKind::Entry ? QVariant(row.packed) : QVariant();
    case 3:
        if (row.kind == RowKind::Parent) {
            return "UP";
        }
        if (row.kind == RowKind::Directory) {
            return "DIR";
        }
        if (row.kind == RowKind::DirectoryLink) {
            return "LNKD";
        }
        if (row.ftype == 'l') {
            return "LNK";
        }
        return QString(row.ftype);
    case 4:
        return row.kind == RowKind::Entry ? QVariant(row.subgroup) : QVariant();
    case 5:
        return row.kind == RowKind::Entry ? QVariant(row.mach) : QVariant();
    case 6:
        return row.kind == RowKind::Entry ? QVariant(row.offset) : QVariant();
    default:
        return {};
    }
}

QVariant FileTableModel::headerData(int section, Qt::Orientation orientation, int role) const {
    if (orientation != Qt::Horizontal || role != Qt::DisplayRole) {
        return QAbstractTableModel::headerData(section, orientation, role);
    }

    switch (section) {
    case 0:
        return "Name";
    case 1:
        return "Size";
    case 2:
        return "Packed";
    case 3:
        return "Type";
    case 4:
        return "Subgroup";
    case 5:
        return "Mach";
    case 6:
        return "Offset";
    default:
        return {};
    }
}

void FileTableModel::setEntries(QVector<swcore::FileEntry> entries) {
    beginResetModel();
    m_entries = std::move(entries);
    m_cachedPaths.clear();
    m_cachedPaths.reserve(m_entries.size());
    for (const swcore::FileEntry &e : m_entries) {
        CachedEntryPath c;
        c.fullPath = normalizedPath(e.fname);
        c.parentPath = parentOf(c.fullPath);
        c.baseName = baseNameOf(c.fullPath);
        c.baseNameLower = c.baseName.toLower();
        m_cachedPaths.push_back(std::move(c));
    }
    m_currentDir.clear();
    rebuildFiltered();
    rebuildRows();
    endResetModel();
}

void FileTableModel::setFilters(const QString &mask, const QString &filter, const QString &machTarget) {
    const QString normalized = mask.trimmed().isEmpty() ? "*" : mask.trimmed();
    const QString normalizedFilter = filter.trimmed();
    const QString normalizedMach = machTarget.trimmed().isEmpty() ? "*" : machTarget.trimmed();
    const bool subgroupChanged = normalized != m_subgroupMask;
    const bool nameChanged = normalizedFilter != m_nameFilter;
    const bool machChanged = normalizedMach != m_machFilter;
    if (!subgroupChanged && !nameChanged && !machChanged) {
        return;
    }

    beginResetModel();
    if (subgroupChanged) {
        m_subgroupMask = normalized;
        m_subgroupRegex = QRegularExpression(QRegularExpression::wildcardToRegularExpression(m_subgroupMask));
    }
    if (nameChanged) {
        m_nameFilter = normalizedFilter;
        m_nameFilterLower = normalizedFilter.toLower();
    }
    if (machChanged) {
        m_machFilter = normalizedMach;
        parseMachTargets(m_machFilter, &m_machIncludeTargets, &m_machExcludeTargets);
    }
    if (subgroupChanged || machChanged) {
        rebuildFiltered();
    }
    rebuildRows();
    endResetModel();
}

void FileTableModel::setSubgroupMask(const QString &mask) {
    setFilters(mask, m_nameFilter, m_machFilter);
}

void FileTableModel::setNameFilter(const QString &filter) {
    setFilters(m_subgroupMask, filter, m_machFilter);
}

void FileTableModel::setMachFilter(const QString &machTarget) {
    setFilters(m_subgroupMask, m_nameFilter, machTarget);
}

void FileTableModel::setCurrentDirectory(const QString &relPath) {
    const QString normalized = normalizedPath(relPath);
    if (normalized == m_currentDir) {
        return;
    }
    beginResetModel();
    m_currentDir = normalized;
    rebuildRows();
    endResetModel();
}

QString FileTableModel::currentDirectory() const {
    return m_currentDir;
}

bool FileTableModel::canGoUp() const {
    return !m_currentDir.isEmpty();
}

void FileTableModel::goUp() {
    if (!canGoUp()) {
        return;
    }
    setCurrentDirectory(parentOf(m_currentDir));
}

FileTableModel::RowKind FileTableModel::rowKind(int row) const {
    if (row < 0 || row >= m_rows.size()) {
        return RowKind::Entry;
    }
    return m_rows.at(row).kind;
}

QString FileTableModel::rowPath(int row) const {
    if (row < 0 || row >= m_rows.size()) {
        return {};
    }
    const RowItem &item = m_rows.at(row);
    if (!item.navigatePath.isEmpty()) {
        return item.navigatePath;
    }
    return item.relPath;
}

QString FileTableModel::rowSourcePath(int row) const {
    if (row < 0 || row >= m_rows.size()) {
        return {};
    }
    return m_rows.at(row).relPath;
}

QVector<swcore::FileEntry> FileTableModel::entriesForRows(const QModelIndexList &rows) const {
    QSet<int> indexes;
    for (const QModelIndex &idx : rows) {
        if (!idx.isValid() || idx.row() < 0 || idx.row() >= m_rows.size()) {
            continue;
        }
        const RowItem &row = m_rows.at(idx.row());
        if (row.kind == RowKind::Parent) {
            continue;
        }
        if (row.kind == RowKind::DirectoryLink) {
            if (row.entryIndex >= 0) {
                indexes.insert(row.entryIndex);
            }
            continue;
        }
        if (row.kind == RowKind::Entry) {
            if (row.entryIndex >= 0) {
                indexes.insert(row.entryIndex);
            }
            continue;
        }

        for (int entryIndex : m_filteredIndexes) {
            const CachedEntryPath &path = m_cachedPaths.at(entryIndex);
            if (isUnderOrEqual(path.fullPath, row.relPath)) {
                indexes.insert(entryIndex);
            }
        }
    }

    return entriesByIndexes(indexes);
}

QVector<swcore::FileEntry> FileTableModel::entriesInCurrentTree() const {
    QSet<int> indexes;
    for (int entryIndex : m_filteredIndexes) {
        const CachedEntryPath &path = m_cachedPaths.at(entryIndex);
        if (m_currentDir.isEmpty() || isUnderOrEqual(path.fullPath, m_currentDir)) {
            indexes.insert(entryIndex);
        }
    }
    return entriesByIndexes(indexes);
}

int FileTableModel::totalFilteredEntryCount() const {
    return m_filteredIndexes.size();
}

QStringList FileTableModel::machConflictSummaries(const QVector<swcore::FileEntry> &entries) {
    QMap<QString, int> countByPath;
    QMap<QString, QStringList> machValuesByPath;

    for (const swcore::FileEntry &entry : entries) {
        const QString mach = entry.machExpr.trimmed();
        if (mach.isEmpty()) {
            continue;
        }

        const QString path = normalizedPath(entry.fname);
        if (path.isEmpty()) {
            continue;
        }

        countByPath[path] += 1;
        QStringList &machValues = machValuesByPath[path];
        if (!machValues.contains(mach)) {
            machValues.push_back(mach);
        }
    }

    QStringList summaries;
    for (auto it = countByPath.cbegin(); it != countByPath.cend(); ++it) {
        if (it.value() < 2) {
            continue;
        }
        summaries.push_back(QString("/%1: %2").arg(it.key(), machValuesByPath.value(it.key()).join(", ")));
    }
    return summaries;
}

QString FileTableModel::normalizedPath(const QString &path) {
    QString p = path;
    p.replace('\\', '/');
    while (p.startsWith('/')) {
        p.remove(0, 1);
    }

    QStringList out;
    const QStringList segs = p.split('/', Qt::SkipEmptyParts);
    for (const QString &seg : segs) {
        if (seg == ".") {
            continue;
        }
        if (seg == "..") {
            if (!out.isEmpty()) {
                out.removeLast();
            }
            continue;
        }
        out.push_back(seg);
    }
    return out.join('/');
}

QString FileTableModel::compactWhitespace(const QString &text) {
    QString compact;
    compact.reserve(text.size());
    for (const QChar ch : text) {
        if (!ch.isSpace()) {
            compact.push_back(ch);
        }
    }
    return compact;
}

QString FileTableModel::parentOf(const QString &path) {
    const int slash = path.lastIndexOf('/');
    if (slash < 0) {
        return {};
    }
    return path.left(slash);
}

QString FileTableModel::baseNameOf(const QString &path) {
    const int slash = path.lastIndexOf('/');
    if (slash < 0) {
        return path;
    }
    return path.mid(slash + 1);
}

QString FileTableModel::resolveLinkPath(const QString &baseDir, const QString &target) {
    QString t = target;
    t.replace('\\', '/');
    if (t.isEmpty()) {
        return {};
    }
    if (t.startsWith('/')) {
        return normalizedPath(t);
    }
    if (baseDir.isEmpty()) {
        return normalizedPath(t);
    }
    return normalizedPath(baseDir + "/" + t);
}

bool FileTableModel::isUnderOrEqual(const QString &path, const QString &dir) {
    if (dir.isEmpty()) {
        return true;
    }
    return path == dir || path.startsWith(dir + "/");
}

bool FileTableModel::isUnder(const QString &path, const QString &dir) {
    if (dir.isEmpty()) {
        return !path.isEmpty();
    }
    return path.startsWith(dir + "/");
}

FileTableModel::MachTarget FileTableModel::parseMachTarget(const QString &text) {
    MachTarget target;
    target.text = text;
    target.compactText = compactWhitespace(text);
    target.hasWildcard = text.contains('*') || text.contains('?');

    const QStringList termTexts = text.split(QRegularExpression("\\s+"), Qt::SkipEmptyParts);
    target.terms.reserve(termTexts.size());
    for (const QString &termText : termTexts) {
        MachTerm term;
        term.text = termText;
        term.compactText = compactWhitespace(termText);
        term.hasWildcard = termText.contains('*') || termText.contains('?');
        term.regex = QRegularExpression(QRegularExpression::wildcardToRegularExpression(termText));
        target.terms.push_back(std::move(term));
    }
    return target;
}

void FileTableModel::parseMachTargets(const QString &filter,
                                      QVector<MachTarget> *includeTargets,
                                      QVector<MachTarget> *excludeTargets) {
    if (includeTargets) {
        includeTargets->clear();
    }
    if (excludeTargets) {
        excludeTargets->clear();
    }

    const QString normalized = filter.trimmed();
    if (normalized.isEmpty() || normalized == "*") {
        return;
    }

    const QStringList parts = normalized.split(',');
    for (const QString &part : parts) {
        QString text = part.trimmed();
        if (text.isEmpty()) {
            continue;
        }

        const bool excluded = text.startsWith('!');
        if (excluded) {
            text.remove(0, 1);
            text = text.trimmed();
            if (text.isEmpty()) {
                continue;
            }
        }

        if (excluded) {
            if (excludeTargets) {
                excludeTargets->push_back(parseMachTarget(text));
            }
        } else if (includeTargets) {
            includeTargets->push_back(parseMachTarget(text));
        }
    }
}

QVector<swcore::FileEntry> FileTableModel::entriesByIndexes(const QSet<int> &indexes) const {
    QList<int> sorted = indexes.values();
    std::sort(sorted.begin(), sorted.end());

    QVector<swcore::FileEntry> out;
    out.reserve(sorted.size());
    for (int idx : sorted) {
        out.push_back(m_entries.at(idx));
    }
    return out;
}

bool FileTableModel::isMachFilterActive() const {
    return !m_machIncludeTargets.isEmpty() || !m_machExcludeTargets.isEmpty();
}

bool FileTableModel::machExprMatchesTargets(const QVector<MachTarget> &targets, const QString &machExpr) const {
    const QString mach = machExpr.trimmed();
    if (mach.isEmpty() || targets.isEmpty()) {
        return false;
    }

    const QString compactMach = compactWhitespace(mach);
    const QStringList machTokens = mach.split(QRegularExpression("\\s+"), Qt::SkipEmptyParts);

    for (const MachTarget &target : targets) {
        if (target.terms.isEmpty()) {
            continue;
        }

        bool allTermsMatch = true;
        for (const MachTerm &term : target.terms) {
            if (!machTermMatches(term, mach, compactMach, machTokens)) {
                allTermsMatch = false;
                break;
            }
        }
        if (allTermsMatch) {
            return true;
        }

        if (!target.hasWildcard && !target.compactText.isEmpty() &&
            compactMach.contains(target.compactText, Qt::CaseInsensitive)) {
            return true;
        }
    }

    return false;
}

bool FileTableModel::machTermMatches(const MachTerm &term,
                                     const QString &machExpr,
                                     const QString &compactMachExpr,
                                     const QStringList &machTokens) const {
    if (term.text.isEmpty()) {
        return true;
    }

    if (term.hasWildcard) {
        if (!term.regex.isValid()) {
            return machExpr.contains(term.text, Qt::CaseInsensitive);
        }

        if (term.regex.match(machExpr).hasMatch()) {
            return true;
        }
        for (const QString &token : machTokens) {
            if (term.regex.match(token).hasMatch()) {
                return true;
            }
        }
        return false;
    }

    if (machExpr.contains(term.text, Qt::CaseInsensitive)) {
        return true;
    }

    return !term.compactText.isEmpty() &&
           compactMachExpr.contains(term.compactText, Qt::CaseInsensitive);
}

bool FileTableModel::matchesMachFilter(const swcore::FileEntry &entry) const {
    if (!isMachFilterActive()) {
        return true;
    }

    if (entry.machExpr.trimmed().isEmpty()) {
        return true;
    }

    if (machExprMatchesTargets(m_machExcludeTargets, entry.machExpr)) {
        return false;
    }

    if (m_machIncludeTargets.isEmpty()) {
        return true;
    }

    return machExprMatchesTargets(m_machIncludeTargets, entry.machExpr);
}

void FileTableModel::rebuildFiltered() {
    QVector<int> candidates;
    candidates.reserve(m_entries.size());
    for (int i = 0; i < m_entries.size(); ++i) {
        const swcore::FileEntry &e = m_entries.at(i);
        if (m_subgroupRegex.isValid() && !m_subgroupRegex.match(e.subgroup).hasMatch()) {
            continue;
        }
        if (!matchesMachFilter(e)) {
            continue;
        }
        candidates.push_back(i);
    }

    if (!isMachFilterActive()) {
        m_filteredIndexes = std::move(candidates);
        return;
    }

    QSet<QString> pathsWithMachSpecificMatch;
    for (int idx : candidates) {
        const QString key = m_cachedPaths.at(idx).fullPath;
        if (key.isEmpty()) {
            continue;
        }
        if (!m_entries.at(idx).machExpr.trimmed().isEmpty()) {
            pathsWithMachSpecificMatch.insert(key);
        }
    }

    m_filteredIndexes.clear();
    m_filteredIndexes.reserve(candidates.size());
    for (int idx : candidates) {
        const QString key = m_cachedPaths.at(idx).fullPath;
        if (key.isEmpty()) {
            m_filteredIndexes.push_back(idx);
            continue;
        }

        const bool isCommon = m_entries.at(idx).machExpr.trimmed().isEmpty();
        if (!isCommon || !pathsWithMachSpecificMatch.contains(key)) {
            m_filteredIndexes.push_back(idx);
        }
    }
}

void FileTableModel::rebuildRows() {
    m_rows.clear();

    if (canGoUp()) {
        RowItem up;
        up.kind = RowKind::Parent;
        up.relPath = parentOf(m_currentDir);
        m_rows.push_back(up);
    }

    QSet<QString> dirsFromNameMatches;
    if (!m_nameFilterLower.isEmpty()) {
        for (int idx : m_filteredIndexes) {
            const CachedEntryPath &path = m_cachedPaths.at(idx);
            if (!path.baseNameLower.contains(m_nameFilterLower)) {
                continue;
            }
            QString ancestor = path.parentPath;
            while (!ancestor.isEmpty()) {
                dirsFromNameMatches.insert(ancestor);
                ancestor = parentOf(ancestor);
            }
        }
    }

    QMap<QString, RowItem> dirRows;
    QVector<RowItem> fileRows;
    QSet<QString> knownDirs;

    for (int idx : m_filteredIndexes) {
        const CachedEntryPath &path = m_cachedPaths.at(idx);
        if (path.fullPath.isEmpty()) {
            continue;
        }
        QString anc = path.parentPath;
        while (!anc.isEmpty()) {
            knownDirs.insert(anc);
            anc = parentOf(anc);
        }
        if (m_entries.at(idx).ftype == 'd') {
            knownDirs.insert(path.fullPath);
        }
    }

    for (int idx : m_filteredIndexes) {
        const swcore::FileEntry &entry = m_entries.at(idx);
        const CachedEntryPath &path = m_cachedPaths.at(idx);
        const QString &fullPath = path.fullPath;
        if (fullPath.isEmpty()) {
            continue;
        }

        const QString &parent = path.parentPath;
        const QString &base = path.baseName;

        if (parent == m_currentDir) {
            if (entry.ftype == 'd') {
                bool show = m_nameFilterLower.isEmpty() || path.baseNameLower.contains(m_nameFilterLower) ||
                            dirsFromNameMatches.contains(fullPath);
                if (!show) {
                    continue;
                }
                if (!dirRows.contains(base)) {
                    RowItem row;
                    row.kind = RowKind::Directory;
                    row.name = base;
                    row.relPath = fullPath;
                    row.ftype = 'd';
                    dirRows.insert(base, row);
                }
            } else {
                if (!m_nameFilterLower.isEmpty() && !path.baseNameLower.contains(m_nameFilterLower)) {
                    continue;
                }
                RowItem row;
                row.kind = RowKind::Entry;
                row.name = base;
                row.relPath = fullPath;
                row.navigatePath = fullPath;
                row.entryIndex = idx;
                row.size = entry.size;
                row.packed = entry.cmpsize;
                row.payload = entry.payloadSize;
                row.subgroup = entry.subgroup;
                row.mach = entry.machExpr;
                row.offset = entry.offset;
                row.ftype = entry.ftype;
                row.linkTarget = entry.symval;
                if (entry.ftype == 'l') {
                    const QString resolved = resolveLinkPath(path.parentPath, entry.symval);
                    if (!resolved.isEmpty()) {
                        row.navigatePath = resolved;
                        if (knownDirs.contains(resolved)) {
                            row.kind = RowKind::DirectoryLink;
                        }
                    }
                }
                fileRows.push_back(row);
            }
            continue;
        }

        if (!isUnder(fullPath, m_currentDir)) {
            continue;
        }

        QString remainder = fullPath;
        if (!m_currentDir.isEmpty()) {
            remainder = fullPath.mid(m_currentDir.size() + 1);
        }
        const int slash = remainder.indexOf('/');
        if (slash < 0) {
            continue;
        }

        const QString childName = remainder.left(slash);
        const QString childPath = joinPath(m_currentDir, childName);
        bool show = m_nameFilterLower.isEmpty() || childName.contains(m_nameFilter, Qt::CaseInsensitive) ||
                    dirsFromNameMatches.contains(childPath);
        if (!show) {
            continue;
        }

        if (!dirRows.contains(childName)) {
            RowItem row;
            row.kind = RowKind::Directory;
            row.name = childName;
            row.relPath = childPath;
            row.ftype = 'd';
            dirRows.insert(childName, row);
        }
    }

    for (auto it = dirRows.cbegin(); it != dirRows.cend(); ++it) {
        m_rows.push_back(it.value());
    }

    std::sort(fileRows.begin(), fileRows.end(), [](const RowItem &a, const RowItem &b) {
        return QString::compare(a.name, b.name, Qt::CaseInsensitive) < 0;
    });
    m_rows += fileRows;
}
