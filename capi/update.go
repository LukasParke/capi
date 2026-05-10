package main

import (
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"time"
)

// Self-update support: download the latest release binary from GitHub and
// trigger a systemd restart. Driven from the CLI (capi -update), the web UI
// (POST /api/update), or the JSON API.

const updateRepo = "LukasParke/capi"

var updateHTTPClient = &http.Client{Timeout: 30 * time.Second}

// releaseInfo is the subset of the GitHub releases JSON we care about.
type releaseInfo struct {
	TagName string         `json:"tag_name"`
	Assets  []releaseAsset `json:"assets"`
}

type releaseAsset struct {
	Name               string `json:"name"`
	BrowserDownloadURL string `json:"browser_download_url"`
}

// checkForUpdate queries the GitHub releases API and returns info about the
// latest release. Returns (nil, nil) if the current version is already up to date.
func checkForUpdate() (*releaseInfo, error) {
	url := fmt.Sprintf("https://api.github.com/repos/%s/releases/latest", updateRepo)
	resp, err := updateHTTPClient.Get(url)
	if err != nil {
		return nil, fmt.Errorf("failed to query GitHub: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("GitHub API returned %d", resp.StatusCode)
	}

	var info releaseInfo
	if err := json.NewDecoder(resp.Body).Decode(&info); err != nil {
		return nil, fmt.Errorf("failed to parse release JSON: %w", err)
	}

	if info.TagName == version {
		return nil, nil
	}
	return &info, nil
}

// assetURL finds the download URL for the named asset in a release.
func assetURL(info *releaseInfo, name string) string {
	for _, a := range info.Assets {
		if a.Name == name {
			return a.BrowserDownloadURL
		}
	}
	return ""
}

// binaryAssetName returns the release asset name for the current architecture.
func binaryAssetName() string {
	switch runtime.GOARCH {
	case "arm64":
		return "capi-linux-arm64"
	case "arm":
		return "capi-linux-armv6"
	default:
		return "capi-linux-arm64"
	}
}

// downloadFile downloads a URL to a local file path via temp file + rename.
func downloadFile(url, dest string) error {
	resp, err := updateHTTPClient.Get(url)
	if err != nil {
		return err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("download returned %d", resp.StatusCode)
	}

	tmp := dest + ".tmp"
	f, err := os.Create(tmp)
	if err != nil {
		return err
	}
	if _, err := io.Copy(f, resp.Body); err != nil {
		f.Close()
		os.Remove(tmp)
		return err
	}
	f.Close()

	if err := os.Chmod(tmp, 0755); err != nil {
		os.Remove(tmp)
		return err
	}
	return os.Rename(tmp, dest)
}

// performUpdate downloads the binary for the given release into the current
// install directory.
func performUpdate(info *releaseInfo) error {
	binName := binaryAssetName()
	binURL := assetURL(info, binName)
	if binURL == "" {
		return fmt.Errorf("release %s has no asset %s", info.TagName, binName)
	}

	exe, err := os.Executable()
	if err != nil {
		exe = "/opt/capi/capi"
	}
	installDir := filepath.Dir(exe)

	log.Printf("Downloading %s from %s ...", binName, info.TagName)
	if err := downloadFile(binURL, filepath.Join(installDir, "capi")); err != nil {
		return fmt.Errorf("binary download failed: %w", err)
	}
	return nil
}

// restartService asks systemd to restart the capi service.
func restartService() error {
	cmd := exec.Command("systemctl", "restart", "capi.service")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}

// doSelfUpdate is the CLI entry-point for `capi -update`.
func doSelfUpdate() {
	log.Printf("Current version: %s", version)
	log.Println("Checking for updates...")

	info, err := checkForUpdate()
	if err != nil {
		log.Fatalf("Update check failed: %v", err)
	}
	if info == nil {
		log.Println("Already up to date.")
		os.Exit(0)
	}

	log.Printf("Update available: %s -> %s", version, info.TagName)
	if err := performUpdate(info); err != nil {
		log.Fatalf("Update failed: %v", err)
	}

	log.Println("Update downloaded. Restarting service...")
	if err := restartService(); err != nil {
		log.Printf("Could not restart service: %v (you may need to restart manually)", err)
	}
	os.Exit(0)
}

// updateHandler is the HTTP handler for POST /api/update.
func updateHandler(w http.ResponseWriter, r *http.Request) {
	info, err := checkForUpdate()
	if err != nil {
		respondError(w, http.StatusBadGateway, fmt.Sprintf("Update check failed: %v", err))
		return
	}
	if info == nil {
		respondSuccess(w, "Already up to date", map[string]interface{}{
			"version": version,
		})
		return
	}

	if err := performUpdate(info); err != nil {
		respondError(w, http.StatusInternalServerError, fmt.Sprintf("Update failed: %v", err))
		return
	}

	respondSuccess(w, fmt.Sprintf("Updated to %s, restarting...", info.TagName), map[string]interface{}{
		"old_version": version,
		"new_version": info.TagName,
	})

	go func() {
		time.Sleep(1 * time.Second)
		_ = restartService()
	}()
}
