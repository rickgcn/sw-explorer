#include "rust/cxx.h"

#include "BackendWorker.h"

namespace {

HierarchyKind toQtKind(sw::ObjectKind kind)
{
    switch (kind) {
    case sw::ObjectKind::Product:
        return HierarchyKind::Product;
    case sw::ObjectKind::Image:
        return HierarchyKind::Image;
    case sw::ObjectKind::Subsystem:
        return HierarchyKind::Subsystem;
    }
    Q_UNREACHABLE();
}

QString toQString(const rust::String &value)
{
    return QString::fromUtf8(value.data(), static_cast<qsizetype>(value.size()));
}

QStringList toQStringList(const rust::Vec<rust::String> &values)
{
    QStringList out;
    out.reserve(static_cast<qsizetype>(values.size()));
    for (const rust::String &value : values) {
        out.append(toQString(value));
    }
    return out;
}

HardwareSnapshot toQt(const sw::HardwareDetail &detail)
{
    HardwareSnapshot out;
    out.known = detail.known;
    out.expressions = toQStringList(detail.expressions);
    out.unresolved = toQStringList(detail.unresolved);
    return out;
}

ConditionalFlagSnapshot toQt(const sw::ConditionalFlagDetail &detail)
{
    ConditionalFlagSnapshot out;
    switch (detail.state) {
    case sw::ConditionalFlagState::No:
        out.state = ConditionalFlagState::No;
        break;
    case sw::ConditionalFlagState::Always:
        out.state = ConditionalFlagState::Always;
        break;
    case sw::ConditionalFlagState::Conditional:
        out.state = ConditionalFlagState::Conditional;
        break;
    case sw::ConditionalFlagState::Unresolved:
        out.state = ConditionalFlagState::Unresolved;
        break;
    }
    out.expressions = toQStringList(detail.expressions);
    out.unresolved = toQStringList(detail.unresolved);
    return out;
}

SubsystemFlagsSnapshot toQt(const sw::SubsystemFlagsDetail &detail)
{
    SubsystemFlagsSnapshot out;
    out.known = detail.known;
    out.required = toQt(detail.required);
    out.defaultFlag = toQt(detail.default_flag);
    out.miniroot = toQt(detail.miniroot);
    out.inplace = detail.inplace;
    out.patch = detail.patch;
    out.clientOnly = detail.client_only;
    out.overlay = detail.overlay;
    out.overlayMember = detail.overlay_member;
    return out;
}

RangeSnapshot toQt(const sw::RangeDetail &detail)
{
    RangeSnapshot out;
    out.target = toQString(detail.target);
    out.minVersion = detail.min_version;
    out.maxIsUnbounded = detail.max_is_unbounded;
    out.maxVersion = detail.max_version;
    return out;
}

QList<RangeSnapshot> toQt(const rust::Vec<sw::RangeDetail> &details)
{
    QList<RangeSnapshot> out;
    out.reserve(static_cast<qsizetype>(details.size()));
    for (const sw::RangeDetail &detail : details) {
        out.append(toQt(detail));
    }
    return out;
}

RulesSnapshot toQt(const sw::RulesDetail &detail)
{
    RulesSnapshot out;
    out.known = detail.known;
    for (const sw::PrerequisiteClauseDetail &clause : detail.prerequisites) {
        out.prerequisites.append({toQt(clause.all_of)});
    }
    out.replaces = toQt(detail.replaces);
    out.incompatibilities = toQt(detail.incompatibilities);
    out.updates = toQt(detail.updates);
    out.follows = toQt(detail.follows);
    return out;
}

ProductDetailSnapshot toQt(const sw::ProductDetail &detail)
{
    ProductDetailSnapshot out;
    out.id = detail.id;
    out.name = toQString(detail.name);
    out.title = toQString(detail.title);
    out.descriptorPresent = detail.descriptor_present;
    out.idbPresent = detail.idb_present;
    out.imageCount = detail.image_count;
    out.subsystemCount = detail.subsystem_count;
    out.entryCount = detail.entry_count;
    out.descriptorGeneration = toQString(detail.descriptor_generation);
    out.descriptorLayoutLevel = detail.descriptor_layout_level;
    out.descriptorStamp = detail.descriptor_stamp;
    out.mach = toQt(detail.mach);
    for (const sw::CutpointDetail &cutpoint : detail.cutpoints) {
        out.cutpoints.append({toQString(cutpoint.path), cutpoint.sequence});
    }
    return out;
}

ImageDetailSnapshot toQt(const sw::ImageDetail &detail)
{
    ImageDetailSnapshot out;
    out.id = detail.id;
    out.name = toQString(detail.name);
    out.title = toQString(detail.title);
    out.productName = toQString(detail.product_name);
    out.descriptorPresent = detail.descriptor_present;
    out.idbPresent = detail.idb_present;
    out.versionKnown = detail.version_known;
    out.version = detail.version;
    out.orderKnown = detail.order_known;
    out.order = detail.order;
    out.subsystemCount = detail.subsystem_count;
    out.entryCount = detail.entry_count;
    out.mach = toQt(detail.mach);
    return out;
}

SubsystemDetailSnapshot toQt(const sw::SubsystemDetail &detail)
{
    SubsystemDetailSnapshot out;
    out.id = detail.id;
    out.identity = toQString(detail.identity);
    out.shortName = toQString(detail.short_name);
    out.title = toQString(detail.title);
    out.productName = toQString(detail.product_name);
    out.imageName = toQString(detail.image_name);
    out.imageVersionKnown = detail.image_version_known;
    out.imageVersion = detail.image_version;
    out.descriptorPresent = detail.descriptor_present;
    out.idbPresent = detail.idb_present;
    out.entryCount = detail.entry_count;
    out.mappingKnown = detail.mapping_known;
    out.mapping = toQString(detail.mapping);
    out.mach = toQt(detail.mach);
    out.flags = toQt(detail.flags);
    out.rules = toQt(detail.rules);
    out.autominirootKnown = detail.autominiroot_known;
    out.autominiroot = toQt(detail.autominiroot);
    return out;
}

EntryFileType toQt(sw::EntryFileType fileType)
{
    switch (fileType) {
    case sw::EntryFileType::Regular:
        return EntryFileType::Regular;
    case sw::EntryFileType::Directory:
        return EntryFileType::Directory;
    case sw::EntryFileType::SymbolicLink:
        return EntryFileType::SymbolicLink;
    case sw::EntryFileType::BlockDevice:
        return EntryFileType::BlockDevice;
    case sw::EntryFileType::CharacterDevice:
        return EntryFileType::CharacterDevice;
    case sw::EntryFileType::Fifo:
        return EntryFileType::Fifo;
    case sw::EntryFileType::Other:
        return EntryFileType::Other;
    }
    Q_UNREACHABLE();
}

EntrySummarySnapshot toQt(const sw::EntrySummary &summary)
{
    EntrySummarySnapshot out;
    out.productId = summary.product_id;
    out.entryId = summary.entry_id;
    out.path = toQString(summary.path);
    out.subsystem = toQString(summary.subsystem);
    out.fileType = toQt(summary.file_type);
    out.fileTypeRaw = toQString(summary.file_type_raw);
    out.sizeKnown = summary.size_known;
    out.size = summary.size;
    out.storedSizeKnown = summary.stored_size_known;
    out.storedSize = summary.stored_size;
    out.mach = toQStringList(summary.mach);
    out.unresolvedMach = toQStringList(summary.unresolved_mach);
    return out;
}

EntryListSnapshot toQt(const rust::Vec<sw::EntrySummary> &entries)
{
    EntryListSnapshot out;
    out.reserve(static_cast<qsizetype>(entries.size()));
    for (const sw::EntrySummary &entry : entries) {
        out.append(toQt(entry));
    }
    return out;
}

HardwareCandidatesSnapshot toQt(const rust::Vec<sw::HardwareCandidateSet> &sets)
{
    HardwareCandidatesSnapshot out;
    out.reserve(static_cast<qsizetype>(sets.size()));
    for (const sw::HardwareCandidateSet &set : sets) {
        out.append({toQString(set.attribute), toQStringList(set.values)});
    }
    return out;
}

SelectionSnapshot toQt(const sw::SelectionSnapshot &selection)
{
    SelectionSnapshot out;
    out.selected.reserve(static_cast<qsizetype>(selection.selected.size()));
    for (const sw::SelectionEntryKey &key : selection.selected) {
        out.selected.append({key.product_id, key.entry_id});
    }
    out.conflicts.reserve(static_cast<qsizetype>(selection.conflicts.size()));
    for (const sw::SelectionConflictDetail &conflict : selection.conflicts) {
        SelectionConflictSnapshot item;
        item.path = toQString(conflict.path);
        item.candidates.reserve(static_cast<qsizetype>(conflict.candidates.size()));
        for (const sw::SelectionEntryKey &key : conflict.candidates) {
            item.candidates.append({key.product_id, key.entry_id});
        }
        out.conflicts.append(item);
    }
    return out;
}

EntryDetailSnapshot toQt(const sw::EntryDetail &detail)
{
    EntryDetailSnapshot out;
    out.productId = detail.product_id;
    out.entryId = detail.entry_id;
    out.fileType = toQt(detail.file_type);
    out.fileTypeRaw = toQString(detail.file_type_raw);
    out.mode = detail.mode;
    out.owner = toQString(detail.owner);
    out.group = toQString(detail.group);
    out.path = toQString(detail.path);
    out.rawPath = toQString(detail.raw_path);
    out.sourcePath = toQString(detail.source_path);
    out.subsystem = toQString(detail.subsystem);
    out.sizeKnown = detail.size_known;
    out.size = detail.size;
    out.compressedSizeKnown = detail.compressed_size_known;
    out.compressedSize = detail.compressed_size;
    out.storedSizeKnown = detail.stored_size_known;
    out.storedSize = detail.stored_size;
    out.checksumKnown = detail.checksum_known;
    out.checksum = detail.checksum;
    out.configKnown = detail.config_known;
    out.configMode = toQString(detail.config_mode);
    out.symlinkTargetKnown = detail.symlink_target_known;
    out.symlinkTarget = toQString(detail.symlink_target);
    out.deviceKnown = detail.device_known;
    out.deviceMajor = detail.device_major;
    out.deviceMinor = detail.device_minor;
    out.mach = toQStringList(detail.mach);
    out.unresolvedMach = toQStringList(detail.unresolved_mach);
    out.payloadPresent = detail.payload_present;
    out.payloadImage = toQString(detail.payload_image);
    out.payloadEncodedSizeKnown = detail.payload_encoded_size_known;
    out.payloadEncodedSize = detail.payload_encoded_size;
    out.expectedRecordOffsetKnown = detail.expected_record_offset_known;
    out.expectedRecordOffset = detail.expected_record_offset;
    out.originIdbPath = toQString(detail.origin_idb_path);
    out.originLine = detail.origin_line;
    out.rawIdbLine = toQString(detail.raw_idb_line);
    return out;
}

} // namespace

BackendWorker::BackendWorker(QObject *parent)
    : QObject(parent)
    , m_backend(sw::new_backend())
{
}

void BackendWorker::openDistribution(const QString &path)
{
    // QString -> rust::Str goes through an explicit UTF-8 byte array;
    // the current locale is never involved.
    const QByteArray utf8 = path.toUtf8();
    const rust::Str rustPath(utf8.constData(), static_cast<std::size_t>(utf8.size()));

    // Load into a throwaway candidate: the committed backend stays
    // untouched until the GUI has accepted the candidate's hierarchy.
    rust::Box<sw::Backend> candidate = sw::new_backend();
    try {
        // Open first, then fetch the hierarchy of the candidate: the
        // GUI receives both in a single signal and never has to ask
        // twice.
        const sw::DistributionSummary summary = candidate->open_distribution(rustPath);
        const rust::Vec<sw::HierarchyNode> nodes = candidate->hierarchy();

        // rust:: types stay on this side of the boundary; the snapshot
        // is plain Qt data.
        HierarchySnapshot snapshot;
        snapshot.reserve(static_cast<qsizetype>(nodes.size()));
        for (const sw::HierarchyNode &node : nodes) {
            HierarchyNodeSnapshot item;
            item.id = node.id;
            item.parentId = node.parent_id;
            item.kind = toQtKind(node.kind);
            item.name = toQString(node.name);
            item.title = toQString(node.title);
            item.entryCount = node.entry_count;
            item.descriptorPresent = node.descriptor_present;
            item.idbPresent = node.idb_present;
            snapshot.append(item);
        }

        m_candidate = std::move(candidate);
        emit candidateReady(summary.product_count, summary.diagnostic_count, snapshot);
    } catch (const rust::Error &error) {
        // The candidate is destroyed with this frame; the committed
        // backend was never involved.
        emit distributionOpenFailed(QString::fromUtf8(error.what()));
    }
}

void BackendWorker::commitCandidate()
{
    if (m_candidate.has_value()) {
        m_backend = std::move(*m_candidate);
        m_candidate.reset();
        // The GUI keeps its tree interaction locked until this signal
        // arrives, so no detail query can be issued against object
        // ids the committed backend does not serve yet.
        emit candidateCommitted();
    }
}

void BackendWorker::discardCandidate()
{
    m_candidate.reset();
}

void BackendWorker::detailRequested(quint64 requestId, quint64 objectId, HierarchyKind kind)
{
    // Detail queries only ever talk to the committed backend; a
    // pending candidate is not queryable by definition.
    try {
        switch (kind) {
        case HierarchyKind::Product:
            emit productDetailReady(requestId, toQt(m_backend->product_detail(objectId)));
            break;
        case HierarchyKind::Image:
            emit imageDetailReady(requestId, toQt(m_backend->image_detail(objectId)));
            break;
        case HierarchyKind::Subsystem:
            emit subsystemDetailReady(requestId, toQt(m_backend->subsystem_detail(objectId)));
            break;
        }
    } catch (const rust::Error &error) {
        emit detailFailed(requestId, QString::fromUtf8(error.what()));
    }
}

void BackendWorker::entriesRequested(quint64 requestId, quint64 scopeId)
{
    // Like every query, this only ever talks to the committed
    // backend; a pending candidate is not queryable.
    try {
        emit entriesReady(requestId, toQt(m_backend->entries(scopeId)));
    } catch (const rust::Error &error) {
        emit entriesFailed(requestId, QString::fromUtf8(error.what()));
    }
}

void BackendWorker::searchEntriesRequested(quint64 requestId, const QString &query)
{
    // A search is a whole-distribution query against the committed
    // backend, like every other query; a pending candidate is not
    // queryable. Results and failures reuse the entries signals:
    // both are plain entry lists for the same browser.
    //
    // QString -> rust::Str goes through an explicit UTF-8 byte array;
    // the current locale is never involved.
    const QByteArray utf8 = query.toUtf8();
    const rust::Str rustQuery(utf8.constData(), static_cast<std::size_t>(utf8.size()));
    try {
        emit entriesReady(requestId, toQt(m_backend->search_entries(rustQuery)));
    } catch (const rust::Error &error) {
        emit entriesFailed(requestId, QString::fromUtf8(error.what()));
    }
}

void BackendWorker::entryDetailRequested(quint64 requestId, quint64 productId, quint64 entryId)
{
    try {
        emit entryDetailReady(requestId, toQt(m_backend->entry_detail(productId, entryId)));
    } catch (const rust::Error &error) {
        emit detailFailed(requestId, QString::fromUtf8(error.what()));
    }
}

void BackendWorker::hardwareCandidatesRequested(quint64 requestId)
{
    // Candidate suggestions describe the committed distribution; a
    // pending candidate is not queryable.
    try {
        emit hardwareCandidatesReady(requestId, toQt(m_backend->hardware_candidates()));
    } catch (const rust::Error &error) {
        emit hardwareCandidatesFailed(requestId, QString::fromUtf8(error.what()));
    }
}

void BackendWorker::selectionRequested(quint64 requestId, const HardwareProfileSnapshot &profile)
{
    // Selection is a whole-distribution query against the committed
    // backend, like every other query; a pending candidate is not
    // queryable.
    //
    // QString -> rust::String goes through an explicit UTF-8 byte
    // array; the current locale is never involved.
    rust::Vec<sw::HardwareValue> values;
    values.reserve(static_cast<std::size_t>(profile.size()));
    for (const HardwareValueSnapshot &pair : profile) {
        const QByteArray attribute = pair.attribute.toUtf8();
        const QByteArray value = pair.value.toUtf8();
        values.push_back(sw::HardwareValue{
            rust::String(attribute.constData(), static_cast<std::size_t>(attribute.size())),
            rust::String(value.constData(), static_cast<std::size_t>(value.size()))});
    }
    try {
        emit selectionReady(requestId, toQt(m_backend->select_entries(std::move(values))));
    } catch (const rust::Error &error) {
        emit selectionFailed(requestId, QString::fromUtf8(error.what()));
    }
}
