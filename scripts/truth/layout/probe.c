/* Generated-data confinement probes. Never run against deposited TeX. */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <unistd.h>

#define CHECK(x) do { if (!(x)) { fprintf(stderr, "probe line %d errno %d\n", __LINE__, errno); return 1; } } while (0)

int main(int argc, char **argv) {
    if (argc != 2) return 2;
    puts("probe started"); fflush(stdout);
    if (!strcmp(argv[1], "cpu")) { for (;;) {} }
    if (!strcmp(argv[1], "wall")) { for (;;) sleep(1); }
    if (!strcmp(argv[1], "memory")) {
        void *p = mmap(NULL, 1024UL * 1024 * 1024, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        CHECK(p == MAP_FAILED && errno == ENOMEM);
        puts("memory bounded"); return 0;
    }
    if (!strcmp(argv[1], "quota")) {
        signal(SIGXFSZ, SIG_IGN);
        char bytes[4096]; memset(bytes, 'q', sizeof bytes);
        const char *names[] = {"/output/a", "/output/b"};
        for (unsigned i = 0; i < 2; i++) {
            int fd = open(names[i], O_WRONLY | O_TRUNC | O_CREAT, 0600);
            CHECK(fd >= 0);
            CHECK(write(fd, bytes, sizeof bytes) == sizeof bytes);
            CHECK(write(fd, bytes, sizeof bytes) == -1 && errno == EFBIG);
            CHECK(ftruncate(fd, 8192) == -1 && errno == EFBIG);
            close(fd);
        }
        CHECK(open("/output/third", O_WRONLY | O_CREAT, 0600) == -1);
        puts("aggregate bounded"); return 0;
    }
    CHECK(!strcmp(argv[1], "isolation"));
    CHECK(getpid() == 2);
    CHECK(getenv("SYNTHETIC_SENTINEL") == NULL);
    CHECK(getenv("LD_PRELOAD") == NULL);
    CHECK(fcntl(99, F_GETFD) == -1 && errno == EBADF);
    CHECK(access("/home", F_OK) == -1);
    CHECK(access("/proc/1/root/home", F_OK) == -1);
    CHECK(open("/input/input.txt", O_WRONLY) == -1);
    CHECK(open("/input/../host-only-sentinel", O_RDONLY) == -1);
    CHECK(open("/root-write", O_CREAT | O_WRONLY, 0600) == -1);
    CHECK(open("/dev/new-file", O_CREAT | O_WRONLY, 0600) == -1);
    CHECK(open("/output/new-file", O_CREAT | O_WRONLY, 0600) == -1);
    CHECK(unlink("/output/a") == -1);
    CHECK(rename("/output/a", "/output/moved") == -1);
    CHECK(link("/output/a", "/output/linked") == -1);
    CHECK(symlink("/input/input.txt", "/output/symlink") == -1);
    CHECK(socket(AF_INET, SOCK_STREAM, 0) == -1 && errno == EPERM);
    CHECK(socket(AF_UNIX, SOCK_STREAM, 0) == -1 && errno == EPERM);
    CHECK(fork() == -1 && errno == EPERM);
    CHECK(syscall(SYS_clone3, NULL, 0) == -1 && errno == EPERM);
    CHECK(syscall(SYS_unshare, 0) == -1 && errno == EPERM);
    CHECK(syscall(SYS_mount, NULL, NULL, NULL, 0, NULL) == -1 && errno == EPERM);
    struct rlimit limit = {8192, 8192};
    CHECK(setrlimit(RLIMIT_FSIZE, &limit) == -1 && errno == EPERM);
    int fd = open("/output/a", O_WRONLY | O_TRUNC | O_CREAT, 0600);
    CHECK(fd >= 0 && write(fd, "output\n", 7) == 7); close(fd);
    fd = open("/input/input.txt", O_RDONLY);
    char input[32] = {0};
    CHECK(fd >= 0 && read(fd, input, sizeof input - 1) == 16);
    CHECK(!strcmp(input, "synthetic input\n")); close(fd);
    puts("isolation bounded");
    return 0;
}
