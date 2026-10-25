// Compares two PNG images and reports how much they differ.
//   imagecompare <actual.png> <golden.png> [--mean-tol F] [--pixel-tol N] [--max-bad F] [--diff out.png] [--update]
// Exit code 0: images match within tolerance, 1: mismatch, 2: usage / IO error.
#define STB_IMAGE_IMPLEMENTATION
#include <tinygltf/stb_image.h>
#define STB_IMAGE_WRITE_IMPLEMENTATION
#include <stb/stb_image_write.h>

#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <string>
#include <vector>

int main(int argc, char** argv)
{
	std::vector<std::string> positional;
	double meanTol = 2.0;     // allowed mean absolute difference per channel (0..255)
	int pixelTol = 24;        // per-pixel channel difference above which a pixel counts as "bad"
	double maxBad = 0.02;     // allowed fraction of bad pixels
	std::string diffPath;
	bool update = false;

	for (int i = 1; i < argc; i++) {
		std::string a = argv[i];
		if (a == "--mean-tol" && i + 1 < argc) meanTol = atof(argv[++i]);
		else if (a == "--pixel-tol" && i + 1 < argc) pixelTol = atoi(argv[++i]);
		else if (a == "--max-bad" && i + 1 < argc) maxBad = atof(argv[++i]);
		else if (a == "--diff" && i + 1 < argc) diffPath = argv[++i];
		else if (a == "--update") update = true;
		else positional.push_back(a);
	}
	if (positional.size() != 2) {
		fprintf(stderr, "usage: imagecompare <actual.png> <golden.png> [--mean-tol F] [--pixel-tol N] [--max-bad F] [--diff out.png] [--update]\n");
		return 2;
	}
	const std::string& actualPath = positional[0];
	const std::string& goldenPath = positional[1];

	int aw, ah, ac;
	unsigned char* actual = stbi_load(actualPath.c_str(), &aw, &ah, &ac, 3);
	if (!actual) {
		fprintf(stderr, "cannot read actual image '%s'\n", actualPath.c_str());
		return 2;
	}

	if (update) {
		std::error_code ec;
		std::filesystem::create_directories(std::filesystem::path(goldenPath).parent_path(), ec);
		std::filesystem::copy_file(actualPath, goldenPath, std::filesystem::copy_options::overwrite_existing, ec);
		stbi_image_free(actual);
		if (ec) {
			fprintf(stderr, "cannot update golden image: %s\n", ec.message().c_str());
			return 2;
		}
		printf("golden image updated: %s\n", goldenPath.c_str());
		return 0;
	}

	int gw, gh, gc;
	unsigned char* golden = stbi_load(goldenPath.c_str(), &gw, &gh, &gc, 3);
	if (!golden) {
		fprintf(stderr, "cannot read golden image '%s' (run with --update to create it)\n", goldenPath.c_str());
		stbi_image_free(actual);
		return 2;
	}
	if (aw != gw || ah != gh) {
		fprintf(stderr, "size mismatch: actual %dx%d, golden %dx%d\n", aw, ah, gw, gh);
		stbi_image_free(actual);
		stbi_image_free(golden);
		return 1;
	}

	const size_t pixels = (size_t)aw * ah;
	double sum = 0.0;
	size_t bad = 0;
	std::vector<unsigned char> diff(diffPath.empty() ? 0 : pixels * 3);
	for (size_t p = 0; p < pixels; p++) {
		int maxDiff = 0;
		for (int c = 0; c < 3; c++) {
			int d = abs((int)actual[p * 3 + c] - (int)golden[p * 3 + c]);
			sum += d;
			maxDiff = std::max(maxDiff, d);
			if (!diff.empty()) diff[p * 3 + c] = (unsigned char)std::min(255, d * 4);
		}
		if (maxDiff > pixelTol) bad++;
	}
	double mean = sum / (double)(pixels * 3);
	double badFraction = (double)bad / (double)pixels;

	printf("mean abs diff: %.4f (tol %.4f), bad pixels: %.4f%% (tol %.4f%%)\n", mean, meanTol, badFraction * 100.0, maxBad * 100.0);
	if (!diff.empty()) stbi_write_png(diffPath.c_str(), aw, ah, 3, diff.data(), aw * 3);

	stbi_image_free(actual);
	stbi_image_free(golden);

	if (mean > meanTol || badFraction > maxBad) {
		printf("FAIL: images differ\n");
		return 1;
	}
	printf("OK\n");
	return 0;
}
