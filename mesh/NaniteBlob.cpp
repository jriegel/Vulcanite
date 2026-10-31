#include "NaniteBlob.h"

#include <cstdio>
#include <cstring>
#include <fstream>

namespace NaniteBlob
{
	uint64_t fnv1a(const void* data, size_t size, uint64_t seed)
	{
		const uint8_t* bytes = static_cast<const uint8_t*>(data);
		uint64_t hash = seed;
		for (size_t i = 0; i < size; ++i) {
			hash ^= bytes[i];
			hash *= 0x100000001b3ull;
		}
		return hash;
	}

	uint64_t fnv1aFile(const std::string& path, uint64_t seed)
	{
		std::ifstream file(path, std::ios::binary);
		if (!file.is_open()) {
			return 0;
		}
		uint64_t hash = seed;
		char buffer[65536];
		while (file.read(buffer, sizeof(buffer)) || file.gcount() > 0) {
			hash = fnv1a(buffer, static_cast<size_t>(file.gcount()), hash);
			if (file.gcount() < static_cast<std::streamsize>(sizeof(buffer))) {
				break;
			}
		}
		return hash;
	}

	uint64_t configHash()
	{
		// Every builder parameter that changes the produced data. If any of these
		// change, an existing cache is stale and must be rebuilt.
		const uint32_t params[] = {
			static_cast<uint32_t>(NANITE_BUILDER_VERSION),
			static_cast<uint32_t>(CLUSTER_TARGET_SIZE),
			static_cast<uint32_t>(CLUSTER_MAX_SIZE),
			static_cast<uint32_t>(CLUSTER_GROUP_TARGET_SIZE),
			static_cast<uint32_t>(CLUSTER_GROUP_MAX_SIZE),
		};
		return fnv1a(params, sizeof(params));
	}

	static uint64_t alignUp(uint64_t value, uint64_t alignment)
	{
		return (value + alignment - 1) / alignment * alignment;
	}

	static void addSection(std::vector<NaniteBlobSection>& sections, uint32_t key,
		uint64_t count, uint64_t size, uint64_t offset)
	{
		NaniteBlobSection s;
		s.key = key;
		s.count = static_cast<uint32_t>(count);
		s.offset = offset;
		s.size = size;
		sections.push_back(s);
	}

	static const NaniteBlobSection* findSection(const std::vector<NaniteBlobSection>& sections, uint32_t key)
	{
		for (const auto& s : sections) {
			if (s.key == key) {
				return &s;
			}
		}
		return nullptr;
	}

	bool write(const std::string& path, const Asset& asset, uint64_t source, std::string& error)
	{
		if (asset.vertexStride == 0) {
			error = "blob: vertexStride is 0";
			return false;
		}
		if (asset.clusterCount() != asset.errorCount()) {
			error = "blob: clusterInfo / errorInfo count mismatch";
			return false;
		}

		// Payload sizes, in a fixed section order.
		struct Payload { uint32_t key; const void* data; uint64_t count; uint64_t elemSize; };
		const Payload payloads[] = {
			{ SECTION_VERTICES,       asset.vertices.data(),            asset.vertices.size(), 1 },
			{ SECTION_INDICES,        asset.indices.data(),             asset.indices.size(), sizeof(uint32_t) },
			{ SECTION_CLUSTER_INFO,   asset.clusterInfo.data(),         asset.clusterCount(), sizeof(ClusterInfo) },
			{ SECTION_ERROR_INFO,     asset.errorInfo.data(),           asset.errorCount(),   sizeof(ErrorInfo) },
			{ SECTION_BVH_NODES,      asset.bvhNodes.data(),            asset.bvhNodes.size(), sizeof(BlobBVHNode) },
			{ SECTION_SORTED_CLUSTER_INDICES, asset.sortedClusterIndices.data(), asset.sortedClusterIndices.size(), sizeof(uint32_t) },
			{ SECTION_LOD_CLUSTER_NUMS, asset.lodClusterNums.data(),    asset.lodClusterNums.size(), sizeof(uint32_t) },
			{ SECTION_BVH_ROOTS,        asset.bvhRoots.data(),             asset.bvhRoots.size(), sizeof(uint32_t) },
		};

		std::vector<NaniteBlobSection> sections;
		uint64_t cursor = alignUp(sizeof(NaniteBlobHeader) + sizeof(NaniteBlobSection) * SECTION_COUNT, 256);
		for (const auto& p : payloads) {
			uint64_t size = p.count * p.elemSize;
			addSection(sections, p.key, static_cast<uint32_t>(p.count), size, cursor);
			cursor = alignUp(cursor + size, 256);
		}

		NaniteBlobHeader header;
		header.magic = NANITE_BLOB_MAGIC;
		header.version = NANITE_BLOB_VERSION;
		header.builderVersion = NANITE_BUILDER_VERSION;
		header.sourceHash = source;
		header.configHash = configHash();
		header.lodNums = asset.lodNums;
		header.sectionCount = static_cast<uint32_t>(sections.size());
		header.sizeBVHNodeInfo = static_cast<uint32_t>(sizeof(BVHNodeInfo));
		header.sizeClusterInfo = static_cast<uint32_t>(sizeof(ClusterInfo));
		header.sizeErrorInfo = static_cast<uint32_t>(sizeof(ErrorInfo));
		header.sizeVertex = asset.vertexStride;
		header.sizeBlobBVHNode = static_cast<uint32_t>(sizeof(BlobBVHNode));

		std::ofstream file(path, std::ios::binary | std::ios::trunc);
		if (!file.is_open()) {
			error = "blob: cannot open for write: " + path;
			return false;
		}

		file.write(reinterpret_cast<const char*>(&header), sizeof(header));
		file.write(reinterpret_cast<const char*>(sections.data()), sizeof(NaniteBlobSection) * sections.size());

		// Pad up to the first payload.
		uint64_t written = sizeof(header) + sizeof(NaniteBlobSection) * sections.size();
		uint64_t firstOffset = sections.empty() ? cursor : sections.front().offset;
		for (uint64_t i = written; i < firstOffset; ++i) {
			file.put(0);
		}

		// Write each payload at its recorded offset, padding between them.
		uint64_t pos = firstOffset;
		for (const auto& p : payloads) {
			const NaniteBlobSection* sec = findSection(sections, p.key);
			while (pos < sec->offset) { file.put(0); ++pos; }
			if (sec->size > 0) {
				file.write(static_cast<const char*>(p.data), static_cast<std::streamsize>(sec->size));
				pos += sec->size;
			}
		}

		if (!file.good()) {
			error = "blob: write error: " + path;
			return false;
		}
		return true;
	}

	static bool readRaw(std::ifstream& file, const NaniteBlobSection* sec, std::vector<uint8_t>& out)
	{
		out.clear();
		if (!sec || sec->size == 0) {
			return true;
		}
		file.seekg(static_cast<std::streamoff>(sec->offset));
		if (!file.good()) {
			return false;
		}
		out.resize(static_cast<size_t>(sec->size));
		file.read(reinterpret_cast<char*>(out.data()), static_cast<std::streamsize>(sec->size));
		return file.good();
	}

	static bool readU32s(std::ifstream& file, const NaniteBlobSection* sec, std::vector<uint32_t>& out)
	{
		std::vector<uint8_t> raw;
		if (!readRaw(file, sec, raw)) {
			return false;
		}
		out.resize(raw.size() / sizeof(uint32_t));
		if (!out.empty()) {
			std::memcpy(out.data(), raw.data(), raw.size());
		}
		return true;
	}

	bool read(const std::string& path, Asset& asset, std::string& error, bool strictSourceHash)
	{
		std::ifstream file(path, std::ios::binary);
		if (!file.is_open()) {
			error = "blob: cannot open: " + path;
			return false;
		}

		NaniteBlobHeader header;
		file.read(reinterpret_cast<char*>(&header), sizeof(header));
		if (!file.good()) {
			error = "blob: truncated header: " + path;
			return false;
		}
		if (header.magic != NANITE_BLOB_MAGIC) {
			error = "blob: bad magic: " + path;
			return false;
		}
		if (header.version != NANITE_BLOB_VERSION) {
			error = "blob: version mismatch (file " + std::to_string(header.version) +
				", expected " + std::to_string(NANITE_BLOB_VERSION) + ")";
			return false;
		}
		if (header.sizeClusterInfo != sizeof(ClusterInfo) ||
			header.sizeErrorInfo != sizeof(ErrorInfo) ||
			header.sizeBlobBVHNode != sizeof(BlobBVHNode)) {
			error = "blob: struct size mismatch - rebuild the cache";
			return false;
		}
		if (header.configHash != configHash()) {
			error = "blob: builder config changed - rebuild the cache";
			return false;
		}
		if (strictSourceHash) {
			uint64_t current = fnv1aFile(asset.sourcePath);
			if (current != 0 && current != header.sourceHash) {
				error = "blob: source mesh changed - rebuild the cache";
				return false;
			}
		}

		if (header.sectionCount > SECTION_COUNT * 4) {
			error = "blob: implausible section count";
			return false;
		}
		std::vector<NaniteBlobSection> sections(header.sectionCount);
		file.read(reinterpret_cast<char*>(sections.data()), sizeof(NaniteBlobSection) * sections.size());
		if (!file.good()) {
			error = "blob: truncated section table";
			return false;
		}

		asset.lodNums = header.lodNums;
		asset.sourceHash = header.sourceHash;
		asset.vertexStride = header.sizeVertex;

		if (!readRaw(file, findSection(sections, SECTION_VERTICES), asset.vertices)) {
			error = "blob: failed reading vertices"; return false;
		}
		if (!readU32s(file, findSection(sections, SECTION_INDICES), asset.indices)) {
			error = "blob: failed reading indices"; return false;
		}
		if (!readRaw(file, findSection(sections, SECTION_CLUSTER_INFO), asset.clusterInfo)) {
			error = "blob: failed reading clusterInfo"; return false;
		}
		if (!readRaw(file, findSection(sections, SECTION_ERROR_INFO), asset.errorInfo)) {
			error = "blob: failed reading errorInfo"; return false;
		}
		{
			const NaniteBlobSection* sec = findSection(sections, SECTION_BVH_NODES);
			std::vector<uint8_t> raw;
			if (!readRaw(file, sec, raw)) { error = "blob: failed reading bvhNodes"; return false; }
			asset.bvhNodes.resize(raw.size() / sizeof(BlobBVHNode));
			if (!asset.bvhNodes.empty()) {
				std::memcpy(asset.bvhNodes.data(), raw.data(), raw.size());
			}
		}
		if (!readU32s(file, findSection(sections, SECTION_SORTED_CLUSTER_INDICES), asset.sortedClusterIndices)) {
			error = "blob: failed reading sortedClusterIndices"; return false;
		}
		if (!readU32s(file, findSection(sections, SECTION_LOD_CLUSTER_NUMS), asset.lodClusterNums)) {
			error = "blob: failed reading lodClusterNums"; return false;
		}
		if (!readU32s(file, findSection(sections, SECTION_BVH_ROOTS), asset.bvhRoots)) {
			error = "blob: failed reading bvhRoots"; return false;
		}
		if (asset.bvhNodes.empty() || asset.bvhRoots.empty()) {
			error = "blob: BVH data missing";
			return false;
		}

		if (asset.clusterCount() != asset.errorCount()) {
			error = "blob: clusterInfo / errorInfo count mismatch after read";
			return false;
		}
		return true;
	}

	bool exists(const std::string& path)
	{
		std::ifstream file(path, std::ios::binary);
		return file.is_open();
	}
}
