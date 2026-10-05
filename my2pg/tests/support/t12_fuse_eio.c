#define FUSE_USE_VERSION 31
#include <fuse3/fuse.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
#include <dirent.h>

static char backing_root[PATH_MAX];
static volatile sig_atomic_t fail_next_report_sync;
static volatile sig_atomic_t stall_next_report_sync;

static void arm_eio(int signal_number) {
    (void)signal_number;
    fail_next_report_sync = 1;
}

static void arm_stall(int signal_number) {
    (void)signal_number;
    stall_next_report_sync = 1;
}

static int full_path(const char *path, char output[PATH_MAX]) {
    if (path[0] != '/' || strstr(path, "/../") != NULL || strcmp(path, "/..") == 0) {
        return -EINVAL;
    }
    int written = snprintf(output, PATH_MAX, "%s%s", backing_root, path);
    return written < 0 || written >= PATH_MAX ? -ENAMETOOLONG : 0;
}

static int get_attr(const char *path, struct stat *info, struct fuse_file_info *file) {
    (void)file;
    char full[PATH_MAX];
    int result = full_path(path, full);
    if (result != 0) return result;
    return lstat(full, info) == 0 ? 0 : -errno;
}

static int read_dir(const char *path, void *buffer, fuse_fill_dir_t fill,
                    off_t offset, struct fuse_file_info *file,
                    enum fuse_readdir_flags flags) {
    (void)offset; (void)file; (void)flags;
    char full[PATH_MAX];
    int result = full_path(path, full);
    if (result != 0) return result;
    DIR *directory = opendir(full);
    if (directory == NULL) return -errno;
    struct dirent *entry;
    while ((entry = readdir(directory)) != NULL) {
        if (fill(buffer, entry->d_name, NULL, 0, 0) != 0) break;
    }
    closedir(directory);
    return 0;
}

static int make_dir(const char *path, mode_t mode) {
    char full[PATH_MAX];
    int result = full_path(path, full);
    if (result != 0) return result;
    if (mkdir(full, mode) == 0) return 0;
    fprintf(stderr, "mkdir %s failed: %d\n", path, errno);
    return -errno;
}

static int remove_dir(const char *path) {
    char full[PATH_MAX];
    int result = full_path(path, full);
    if (result != 0) return result;
    return rmdir(full) == 0 ? 0 : -errno;
}

static int remove_file(const char *path) {
    char full[PATH_MAX];
    int result = full_path(path, full);
    if (result != 0) return result;
    return unlink(full) == 0 ? 0 : -errno;
}

static int rename_file(const char *from, const char *to, unsigned int flags) {
    if (flags != 0) return -EINVAL;
    char old_path[PATH_MAX], new_path[PATH_MAX];
    int result = full_path(from, old_path);
    if (result != 0) return result;
    result = full_path(to, new_path);
    if (result != 0) return result;
    if (rename(old_path, new_path) == 0) return 0;
    fprintf(stderr, "rename %s failed: %d\n", from, errno);
    return -errno;
}

static int link_file(const char *from, const char *to) {
    char old_path[PATH_MAX], new_path[PATH_MAX];
    int result = full_path(from, old_path);
    if (result != 0) return result;
    result = full_path(to, new_path);
    if (result != 0) return result;
    if (link(old_path, new_path) == 0) return 0;
    fprintf(stderr, "link %s failed: %d\n", from, errno);
    return -errno;
}

static int open_file(const char *path, struct fuse_file_info *file) {
    char full[PATH_MAX];
    int result = full_path(path, full);
    if (result != 0) return result;
    int descriptor = open(full, file->flags);
    if (descriptor < 0) {
        fprintf(stderr, "open %s failed: %d\n", path, errno);
        return -errno;
    }
    file->fh = (uint64_t)descriptor;
    return 0;
}

static int create_file(const char *path, mode_t mode, struct fuse_file_info *file) {
    char full[PATH_MAX];
    int result = full_path(path, full);
    if (result != 0) return result;
    int descriptor = open(full, file->flags | O_CREAT | O_EXCL, mode);
    if (descriptor < 0) {
        fprintf(stderr, "create %s failed: %d\n", path, errno);
        return -errno;
    }
    file->fh = (uint64_t)descriptor;
    return 0;
}

static int read_file(const char *path, char *buffer, size_t size, off_t offset,
                     struct fuse_file_info *file) {
    (void)path;
    ssize_t count = pread((int)file->fh, buffer, size, offset);
    return count < 0 ? -errno : (int)count;
}

static int write_file(const char *path, const char *buffer, size_t size, off_t offset,
                      struct fuse_file_info *file) {
    (void)path;
    ssize_t count = pwrite((int)file->fh, buffer, size, offset);
    if (count >= 0) return (int)count;
    fprintf(stderr, "write failed: %d\n", errno);
    return -errno;
}

static int sync_file(const char *path, int data_only, struct fuse_file_info *file) {
    (void)data_only;
    if (strcmp(path, "/probe-eio") == 0) return -EIO;
    if (stall_next_report_sync && strstr(path, "/report.json.tmp") != NULL) {
        stall_next_report_sync = 0;
        char marker[PATH_MAX], release[PATH_MAX];
        if (snprintf(marker, sizeof(marker), "%s/stalled", backing_root) >= (int)sizeof(marker) ||
            snprintf(release, sizeof(release), "%s/release", backing_root) >= (int)sizeof(release)) {
            return -ENAMETOOLONG;
        }
        int marker_fd = open(marker, O_WRONLY | O_CREAT | O_TRUNC, 0600);
        if (marker_fd < 0) return -errno;
        close(marker_fd);
        for (int attempt = 0; attempt < 1000; attempt++) {
            if (access(release, F_OK) == 0) break;
            usleep(10000);
        }
        if (access(release, F_OK) != 0) return -ETIMEDOUT;
    }
    if (fail_next_report_sync && strstr(path, "/report.json.tmp") != NULL) {
        fail_next_report_sync = 0;
        return -EIO;
    }
    if (fsync((int)file->fh) == 0) return 0;
    fprintf(stderr, "fsync %s failed: %d\n", path, errno);
    return -errno;
}

static int open_dir(const char *path, struct fuse_file_info *file) {
    char full[PATH_MAX];
    int result = full_path(path, full);
    if (result != 0) return result;
    int descriptor = open(full, O_RDONLY | O_DIRECTORY);
    if (descriptor < 0) return -errno;
    file->fh = (uint64_t)descriptor;
    return 0;
}

static int sync_dir(const char *path, int data_only, struct fuse_file_info *file) {
    (void)data_only;
    if (fsync((int)file->fh) == 0) return 0;
    fprintf(stderr, "fsync directory %s failed: %d\n", path, errno);
    return -errno;
}

static int release_file(const char *path, struct fuse_file_info *file) {
    (void)path;
    return close((int)file->fh) == 0 ? 0 : -errno;
}

static struct fuse_operations operations = {
    .getattr = get_attr,
    .readdir = read_dir,
    .mkdir = make_dir,
    .rmdir = remove_dir,
    .unlink = remove_file,
    .rename = rename_file,
    .link = link_file,
    .open = open_file,
    .create = create_file,
    .read = read_file,
    .write = write_file,
    .fsync = sync_file,
    .opendir = open_dir,
    .fsyncdir = sync_dir,
    .release = release_file,
    .releasedir = release_file,
};

int main(int argc, char **argv) {
    if (argc != 3 || realpath(argv[2], backing_root) == NULL) return 2;
    signal(SIGUSR1, arm_eio);
    signal(SIGUSR2, arm_stall);
    char options[256];
    int length = snprintf(options, sizeof(options),
                          "attr_timeout=0,entry_timeout=0,negative_timeout=0,"
                          "fsname=my2pg-t12-eio-%ld,subtype=my2pg_t12_eio",
                          (long)getpid());
    if (length < 0 || (size_t)length >= sizeof(options)) return 2;
    char *fuse_argv[] = {
        argv[0], "-f", "-s", "-o", options, argv[1]
    };
    return fuse_main(6, fuse_argv, &operations, NULL);
}
