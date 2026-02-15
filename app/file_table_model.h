#pragma once

#include "swcore/types.h"

#include <QAbstractTableModel>
#include <QIcon>
#include <QRegularExpression>
#include <QSet>

class FileTableModel : public QAbstractTableModel {
    Q_OBJECT
public:
    enum class RowKind {
        Parent,
        Directory,
        DirectoryLink,
        Entry
    };

    explicit FileTableModel(QObject *parent = nullptr);

    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    int columnCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role = Qt::DisplayRole) const override;
    QVariant headerData(int section, Qt::Orientation orientation, int role) const override;

    void setEntries(QVector<swcore::FileEntry> entries);
    void setFilters(const QString &mask, const QString &filter);
    void setSubgroupMask(const QString &mask);
    void setNameFilter(const QString &filter);
    void setCurrentDirectory(const QString &relPath);
    QString currentDirectory() const;
    bool canGoUp() const;
    void goUp();

    RowKind rowKind(int row) const;
    QString rowPath(int row) const;
    QString rowSourcePath(int row) const;

    QVector<swcore::FileEntry> entriesForRows(const QModelIndexList &rows) const;
    QVector<swcore::FileEntry> entriesInCurrentTree() const;
    int totalFilteredEntryCount() const;

private:
    struct CachedEntryPath {
        QString fullPath;
        QString parentPath;
        QString baseName;
        QString baseNameLower;
    };

    struct RowItem {
        RowKind kind = RowKind::Entry;
        QString name;
        QString relPath;
        QString navigatePath;
        QString linkTarget;
        int entryIndex = -1;
        qint64 size = 0;
        qint64 packed = 0;
        qint64 payload = 0;
        QString subgroup;
        QString mach;
        qint64 offset = -1;
        QChar ftype;
    };

    static QString normalizedPath(const QString &path);
    static QString parentOf(const QString &path);
    static QString baseNameOf(const QString &path);
    static QString resolveLinkPath(const QString &baseDir, const QString &target);
    static bool isUnderOrEqual(const QString &path, const QString &dir);
    static bool isUnder(const QString &path, const QString &dir);

    QVector<swcore::FileEntry> entriesByIndexes(const QSet<int> &indexes) const;
    void rebuildSubgroupFiltered();
    void rebuildRows();

    QVector<swcore::FileEntry> m_entries;
    QVector<CachedEntryPath> m_cachedPaths;
    QVector<int> m_subgroupFilteredIndexes;
    QVector<RowItem> m_rows;
    QString m_currentDir;
    QString m_subgroupMask = "*";
    QString m_nameFilter;
    QString m_nameFilterLower;
    QRegularExpression m_subgroupRegex{QRegularExpression::wildcardToRegularExpression("*")};
    QIcon m_upIcon;
    QIcon m_dirIcon;
    QIcon m_fileIcon;
    QIcon m_linkIcon;
};
