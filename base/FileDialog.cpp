#include "FileDialog.h"

#include <cstring>
#include <iostream>

#if defined(_WIN32)
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>
#include <commdlg.h>
#pragma comment(lib, "comdlg32.lib")
#endif

namespace
{
#if defined(_WIN32)
	std::wstring utf8ToWide(const std::string& utf8)
	{
		if (utf8.empty()) {
			return std::wstring();
		}
		const int len = MultiByteToWideChar(CP_UTF8, 0, utf8.data(), static_cast<int>(utf8.size()), nullptr, 0);
		std::wstring wide(static_cast<size_t>(len), L'\0');
		MultiByteToWideChar(CP_UTF8, 0, utf8.data(), static_cast<int>(utf8.size()), &wide[0], len);
		return wide;
	}

	std::string wideToUtf8(const wchar_t* wide)
	{
		const int len = WideCharToMultiByte(CP_UTF8, 0, wide, -1, nullptr, 0, nullptr, nullptr);
		if (len <= 0) {
			return std::string();
		}
		std::string utf8(static_cast<size_t>(len - 1), '\0');
		WideCharToMultiByte(CP_UTF8, 0, wide, -1, &utf8[0], len, nullptr, nullptr);
		return utf8;
	}

	// "desc\0pattern\0desc\0pattern\0\0" - the dialog expects a double null terminated block.
	std::vector<wchar_t> buildFilterBlock(const std::vector<vks::FileFilter>& filters)
	{
		std::vector<wchar_t> block;
		for (const auto& filter : filters) {
			const std::wstring description = utf8ToWide(filter.first);
			// ImGui style "glTF (*.gltf)" -> win32 pattern list; keep the description as is and
			// split the pattern on spaces.
			std::wstring pattern;
			for (size_t i = 0; i < filter.second.size(); i++) {
				if (filter.second[i] == ' ') {
					pattern += L';';
				}
				else {
					pattern += static_cast<wchar_t>(filter.second[i]);
				}
			}
			block.insert(block.end(), description.begin(), description.end());
			block.push_back(L'\0');
			block.insert(block.end(), pattern.begin(), pattern.end());
			block.push_back(L'\0');
		}
		block.push_back(L'\0');
		return block;
	}
#endif
}

bool vks::openFileDialog(std::string& outPath, const std::string& title, const std::vector<FileFilter>& filters, const std::string& initialDir)
{
#if defined(_WIN32)
	// MAX_PATH is not enough for long paths, so over-allocate and let the dialog extend the buffer.
	std::vector<wchar_t> pathBuffer(32768, L'\0');

	std::vector<wchar_t> filterBlock = buildFilterBlock(filters);
	std::wstring caption = utf8ToWide(title);
	// Kept alive for the whole call: OPENFILENAME only stores a pointer to it.
	std::wstring initialDirWide = utf8ToWide(initialDir);

	OPENFILENAMEW ofn{};
	ofn.lStructSize = sizeof(ofn);
	ofn.hwndOwner = nullptr;
	ofn.lpstrFilter = filterBlock.empty() ? nullptr : filterBlock.data();
	ofn.lpstrCustomFilter = nullptr;
	ofn.nMaxCustFilter = 0;
	ofn.nFilterIndex = 0;
	ofn.lpstrFile = pathBuffer.data();
	ofn.nMaxFile = static_cast<DWORD>(pathBuffer.size());
	ofn.lpstrFileTitle = nullptr;
	ofn.nMaxFileTitle = 0;
	ofn.lpstrInitialDir = initialDirWide.empty() ? nullptr : initialDirWide.c_str();
	ofn.lpstrTitle = caption.empty() ? nullptr : caption.c_str();
	// OFN_NOCHANGEDIR matters here: the renderer resolves relative asset paths against the process
	// working directory and must not be affected by browsing.
	ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR | OFN_EXPLORER;

	if (!GetOpenFileNameW(&ofn)) {
		const DWORD err = CommDlgExtendedError();
		if (err != 0) {
			std::cerr << "[dialog] file dialog failed with code " << err << std::endl;
		}
		return false;
	}

	outPath = wideToUtf8(pathBuffer.data());
	return !outPath.empty();
#else
	// No native dialog outside Windows. The examples are only built/tested on Windows, so this is
	// deliberately a no-op instead of a half-working custom widget.
	std::cerr << "[dialog] no file dialog available on this platform - use --model <name>" << std::endl;
	(void)outPath;
	(void)title;
	(void)filters;
	(void)initialDir;
	return false;
#endif
}
