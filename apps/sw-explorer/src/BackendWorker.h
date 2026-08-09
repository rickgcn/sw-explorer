#pragma once

#include <QObject>
#include <QString>

#include "sw_gui_bridge.h"

// Owns the Rust backend and runs every call into it. Lives in a
// dedicated QThread; the GUI thread never touches sw::Backend.
class BackendWorker : public QObject
{
    Q_OBJECT

public:
    explicit BackendWorker(QObject *parent = nullptr);

public slots:
    void openDistribution(const QString &path);

signals:
    void distributionOpened(quint64 productCount, quint64 diagnosticCount);
    void distributionOpenFailed(const QString &message);

private:
    rust::Box<sw::Backend> m_backend;
};
