#pragma once

#include "HardwareSnapshot.h"

#include <QDialog>

QT_BEGIN_NAMESPACE
class QPushButton;
class QTableView;
QT_END_NAMESPACE

class HardwareProfileDelegate;
class HardwareProfileModel;

// The hardware profile editor: a table of attribute/value pairs
// describing the simulated target machine.
//
// The profile is a list of hardware facts, not a MACH expression;
// the same attribute may occupy several rows with different values.
// Apply emits the edited profile (empty profile = hardware selection
// disabled); Cancel leaves the applied profile untouched.
class HardwareProfileDialog : public QDialog
{
    Q_OBJECT

public:
    explicit HardwareProfileDialog(QWidget *parent = nullptr);

    // The rows the editor starts from.
    void setProfile(const HardwareProfileSnapshot &profile);
    // Suggestions for the attribute and value comboboxes.
    void setCandidates(const HardwareCandidatesSnapshot &candidates);

signals:
    // The user applied a valid profile; an empty list disables the
    // hardware selection.
    void profileApplied(const HardwareProfileSnapshot &profile);

private slots:
    void addRow();
    void removeSelectedRows();
    void clearRows();
    void apply();

private:
    QTableView *m_view = nullptr;
    HardwareProfileModel *m_model = nullptr;
    HardwareProfileDelegate *m_delegate = nullptr;
    QPushButton *m_addButton = nullptr;
    QPushButton *m_removeButton = nullptr;
    QPushButton *m_clearButton = nullptr;
    QPushButton *m_applyButton = nullptr;
    QPushButton *m_cancelButton = nullptr;
};
