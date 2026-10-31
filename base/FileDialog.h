/*
* Native file open dialog
*
* `external/imgui` only ships core ImGui, so the UI overlay has no file picker. On Windows this
* wraps GetOpenFileNameW; the returned path is always UTF-8 so it can be handed straight to the
* asset loaders (which take std::string).
*/

#pragma once

#include <string>
#include <utility>
#include <vector>

namespace vks
{
	/** @brief A filter row for the dialog, e.g. {"glTF models", "*.gltf *.glb"} */
	using FileFilter = std::pair<std::string, std::string>;

	/**
	* @brief Opens a modal file picker and returns the selected path as UTF-8
	* @param outPath Receives the absolute path on success, untouched on failure
	* @param title Dialog caption
	* @param filters Filter rows; empty means "every file"
	* @param initialDir Directory the dialog opens in (optional)
	* @return false if the user cancelled or no native dialog exists on this platform
	* @note The process working directory is never changed, so relative asset paths keep working
	*/
	bool openFileDialog(std::string& outPath, const std::string& title, const std::vector<FileFilter>& filters = {}, const std::string& initialDir = std::string());
}
