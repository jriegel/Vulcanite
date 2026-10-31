/*
	nanite_bake - the asset pipeline as a standalone tool.

	Runs the builder (glTF -> OpenMesh -> clusters -> cluster groups -> METIS partition ->
	QEM edge collapse -> re-cluster -> per-LOD BVH) and writes the caches the viewer
	consumes next to the source mesh:

		<model>_naniteCache/nanite_info.json   the builder's own description (legacy path)
		<model>_naniteCache/nanite.blob        the final GPU payloads (see mesh/NaniteBlob.h)

	After this tool has run, `pbrtexture --nanite-blob <model>` never touches OpenMesh,
	METIS or JSON - it memcpy's the blob straight into its storage buffers.

	A Vulkan device is created only because `vkglTF::Model::loadFromFile` uploads the
	vertex/index buffers it parses; there is no window, surface or swapchain. `--verify`
	skips Vulkan completely and only re-reads an existing blob, which makes it usable as a
 cheap CI check on baked assets.
*/

#include <chrono>
#include <cstring>
#include <filesystem>
#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>

#include "VulkanDevice.h"
#include "VulkanInitializers.hpp"
#include "VulkanTools.h"
#include "VulkanglTFModel.h"

#include "NaniteBlob.h"
#include "NaniteMesh.h"

namespace
{
	struct Options
	{
		std::vector<std::string> models;
		bool rebuild = false;
		bool jsonOnly = false;
		bool verify = false;
	};

	Options parseArgs(int argc, char** argv)
	{
		Options options;
		for (int i = 1; i < argc; i++) {
			const std::string arg = argv[i];
			if (arg == "--rebuild") {
				options.rebuild = true;
			}
			else if (arg == "--json-only") {
				options.jsonOnly = true;
			}
			else if (arg == "--verify") {
				options.verify = true;
			}
			else if (arg == "--help" || arg == "-h") {
				options.models.clear();
				return options;
			}
			else if (arg.empty() || arg[0] == '-') {
				std::cerr << "Unknown option: " << arg << std::endl;
				std::exit(2);
			}
			else {
				options.models.push_back(arg);
			}
		}
		return options;
	}

	void printUsage()
	{
		std::cout <<
			"nanite_bake - build the nanite caches for one or more glTF models\n\n"
			"usage: nanite_bake [options] <model.gltf> [...]\n\n"
			"  --rebuild     ignore existing caches and rebuild from the glTF\n"
			"  --json-only   write only nanite_info.json, no binary blob\n"
			"  --verify      do not build: re-read the blob of every given model and report\n"
			"                whether it is usable (no Vulkan, no build)\n\n"
			"Caches are written to <model>_naniteCache/ next to the source mesh.\n";
	}

	// The blob is the contract between the two pipeline stages, so re-reading it here proves
	// the bake actually produced something the viewer can consume.
	bool verifyBlob(const std::string& modelFile)
	{
		const std::string path = NaniteMesh::blobPathFor(modelFile);
		if (!NaniteBlob::exists(path)) {
			std::cerr << "[verify] no blob at " << path << std::endl;
			return false;
		}
		NaniteBlob::Asset asset;
		std::string error;
		// Strict: a source hash mismatch means the glTF changed underneath the blob, which is
		// exactly what a bake is supposed to fix.
		if (!NaniteBlob::read(path, asset, error, true)) {
			std::cerr << "[verify] " << path << ": " << error << std::endl;
			return false;
		}
		std::error_code ec;
		const auto fileSize = std::filesystem::file_size(path, ec);
		std::cout << "[verify] " << path << " ok - "
			<< asset.lodNums << " LODs, " << asset.vertexCount() << " verts, "
			<< asset.indices.size() << " indices, " << asset.clusterCount() << " clusters, "
			<< asset.bvhNodes.size() << " bvh nodes, " << fileSize << " bytes" << std::endl;
		return true;
	}

	/*
	* Minimal headless Vulkan setup: instance without any WSI extension, first (preferring a
	* discrete) physical device, logical device without the swapchain extension. Only needed to
	* let vkglTF parse and upload a model; nothing is ever rendered.
	*/
	struct HeadlessDevice
	{
		VkInstance       instance = VK_NULL_HANDLE;
		VkPhysicalDevice physicalDevice = VK_NULL_HANDLE;
		vks::VulkanDevice* device = nullptr;
		VkQueue            transferQueue = VK_NULL_HANDLE;

		void create()
		{
			VkApplicationInfo appInfo{};
			appInfo.sType = VK_STRUCTURE_TYPE_APPLICATION_INFO;
			appInfo.pApplicationName = "nanite_bake";
			appInfo.pEngineName = "nanite_bake";
			appInfo.apiVersion = VK_API_VERSION_1_2;
			// No WSI extension: a surface would require a window, and nothing is presented here.
			VkInstanceCreateInfo instanceCI{};
			instanceCI.sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO;
			instanceCI.pApplicationInfo = &appInfo;
			VK_CHECK_RESULT(vkCreateInstance(&instanceCI, nullptr, &instance));

			uint32_t count = 0;
			VK_CHECK_RESULT(vkEnumeratePhysicalDevices(instance, &count, nullptr));
			if (count == 0) {
				throw std::runtime_error("no Vulkan physical device found");
			}
			std::vector<VkPhysicalDevice> devices(count);
			VK_CHECK_RESULT(vkEnumeratePhysicalDevices(instance, &count, devices.data()));
			physicalDevice = devices[0];
			for (VkPhysicalDevice device : devices) {
				VkPhysicalDeviceProperties properties{};
				vkGetPhysicalDeviceProperties(device, &properties);
				if (properties.deviceType == VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU) {
					physicalDevice = device;
					break;
				}
			}
			VkPhysicalDeviceProperties properties{};
			vkGetPhysicalDeviceProperties(physicalDevice, &properties);
			std::cout << "[bake] device: " << properties.deviceName << std::endl;

			device = new vks::VulkanDevice(physicalDevice);
			// useSwapChain = false: no surface exists, so VK_KHR_swapchain must not be requested.
			VkPhysicalDeviceFeatures enabledFeatures{};
			VK_CHECK_RESULT(device->createLogicalDevice(
				enabledFeatures, {}, nullptr, false,
				VK_QUEUE_GRAPHICS_BIT | VK_QUEUE_TRANSFER_BIT));
			vkGetDeviceQueue(device->logicalDevice, device->queueFamilyIndices.transfer, 0, &transferQueue);
		}

		void destroy()
		{
			if (device) {
				vkDeviceWaitIdle(device->logicalDevice);
				delete device;
				device = nullptr;
			}
			if (instance) {
				vkDestroyInstance(instance, nullptr);
				instance = VK_NULL_HANDLE;
			}
		}
	};
}  // namespace

int main(int argc, char** argv)
{
	const Options options = parseArgs(argc, argv);
	if (options.models.empty()) {
		printUsage();
		return options.verify || argc < 2 ? 2 : 0;
	}

	if (options.verify) {
		bool ok = true;
		for (const std::string& model : options.models) {
			ok = verifyBlob(model) && ok;
		}
		return ok ? 0 : 1;
	}

	HeadlessDevice headless;
	try {
		headless.create();
	}
	catch (const std::exception& e) {
		std::cerr << "[bake] " << e.what() << std::endl;
		headless.destroy();
		return 1;
	}

	int failures = 0;
	for (const std::string& modelFile : options.models) {
		std::error_code ec;
		if (!std::filesystem::exists(modelFile, ec)) {
			std::cerr << "[bake] no such file: " << modelFile << std::endl;
			failures++;
			continue;
		}

		// `setModelPath` only stores the pointer, so the string has to outlive the call.
		const std::string modelDir = modelFile.substr(0, modelFile.find_last_of("\\/") + 1);

		vkglTF::Model model;
		// IMPORTANT: these flags must stay identical to the viewer's `glTFLoadingFlags`
		// (examples/pbrtexture/pbrtexture.cpp). `loadFromFile` rewrites `vertexBuffer` in
		// place - FlipY negates pos.y/normal.y and PreTransformVertices applies the node
		// hierarchy matrix - and the builder consumes exactly those vertices, so baking with
		// a different flag set produces a mirrored/misplaced mesh that still passes every
		// hash check. Only DontLoadImages is added: the builder never reads a texture and
		// uploading images would need descriptors the tool does not have.
		const uint32_t glTFLoadingFlags =
			vkglTF::FileLoadingFlags::PreTransformVertices
			| vkglTF::FileLoadingFlags::PreMultiplyVertexColors
			| vkglTF::FileLoadingFlags::FlipY
			| vkglTF::FileLoadingFlags::DontLoadImages;
		model.loadFromFile(modelFile, headless.device, headless.transferQueue, glTFLoadingFlags);

		NaniteMesh naniteMesh;
		naniteMesh.setModelPath(modelDir.c_str());
		naniteMesh.loadvkglTFModel(model);

		NaniteMesh::BlobMode mode = NaniteMesh::BlobMode::Auto;
		if (options.jsonOnly) {
			mode = NaniteMesh::BlobMode::JsonOnly;
		}
		else if (options.rebuild) {
			mode = NaniteMesh::BlobMode::Rebuild;
		}

		const auto start = std::chrono::steady_clock::now();
		naniteMesh.initNaniteInfo(modelFile, true, mode);
		const auto seconds = std::chrono::duration<double>(std::chrono::steady_clock::now() - start).count();

		std::cout << "[bake] " << modelFile << ": " << naniteMesh.lodNums << " LODs in "
			<< seconds << " s -> " << NaniteMesh::cachePathFor(modelFile) << std::endl;

		// Release the model before the device: ~vkglTF::Model uses it.
		model.clear();

		if (mode != NaniteMesh::BlobMode::JsonOnly && !verifyBlob(modelFile)) {
			failures++;
		}
	}

	headless.destroy();
	return failures == 0 ? 0 : 1;
}
