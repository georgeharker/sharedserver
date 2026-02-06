/*
 * sharedserver-watcher.c - Efficient process watcher using waitpid
 *
 * This is an optional optimized watcher that uses waitpid() to efficiently
 * wait for a process to exit, rather than polling with kill -0.
 *
 * Compile:
 *   gcc -O2 -o sharedserver-watcher sharedserver-watcher.c
 *
 * Usage:
 *   sharedserver-watcher <pid> <lockfile> <lockfile.lock>
 *
 * This program:
 * 1. Waits for the specified PID to exit (blocking, no CPU usage)
 * 2. Atomically decrements refcount in lockfile
 * 3. Removes lockfile if refcount reaches 0
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/wait.h>
#include <sys/file.h>
#include <errno.h>
#include <signal.h>

#define MAX_PATH 1024
#define MAX_JSON 4096

/* Simple JSON manipulation - just for refcount */
static int read_refcount(const char *lockfile) {
    FILE *f = fopen(lockfile, "r");
    if (!f) return -1;

    char json[MAX_JSON];
    if (!fgets(json, sizeof(json), f)) {
        fclose(f);
        return -1;
    }
    fclose(f);

    /* Hacky JSON parsing - look for "refcount": <number> */
    char *refcount_str = strstr(json, "\"refcount\"");
    if (!refcount_str) return -1;

    refcount_str = strchr(refcount_str, ':');
    if (!refcount_str) return -1;

    return atoi(refcount_str + 1);
}

static int write_refcount(const char *lockfile, int refcount) {
    FILE *f_in = fopen(lockfile, "r");
    if (!f_in) return -1;

    char json[MAX_JSON];
    size_t len = fread(json, 1, sizeof(json) - 1, f_in);
    fclose(f_in);
    json[len] = '\0';

    /* Replace refcount value */
    char *refcount_str = strstr(json, "\"refcount\"");
    if (!refcount_str) return -1;

    char *colon = strchr(refcount_str, ':');
    if (!colon) return -1;

    char *comma = strchr(colon, ',');
    char *brace = strchr(colon, '}');
    char *end = (comma && (!brace || comma < brace)) ? comma : brace;
    if (!end) return -1;

    /* Build new JSON */
    char new_json[MAX_JSON];
    size_t prefix_len = colon - json + 1;
    snprintf(new_json, sizeof(new_json), "%.*s %d%s",
             (int)prefix_len, json, refcount, end);

    FILE *f_out = fopen(lockfile, "w");
    if (!f_out) return -1;

    fputs(new_json, f_out);
    fclose(f_out);
    return 0;
}

/* Wait for process using kill -0 polling (fallback if not our child) */
static void wait_polling(pid_t pid) {
    /* Exponential backoff from 100ms to 5s */
    unsigned long sleep_us = 100000; /* 100ms */
    const unsigned long max_sleep_us = 5000000; /* 5s */

    while (kill(pid, 0) == 0) {
        usleep(sleep_us);
        sleep_us = (unsigned long)(sleep_us * 1.5);
        if (sleep_us > max_sleep_us) sleep_us = max_sleep_us;
    }
}

/* Try to wait using waitpid (only works if we're a child of init and target is our sibling) */
static int try_waitpid(pid_t pid) {
    int status;
    pid_t result;

    /* Non-blocking check if we can wait on this PID */
    result = waitpid(pid, &status, WNOHANG);

    if (result == -1 && errno == ECHILD) {
        /* Not our child, can't use waitpid */
        return 0;
    }

    if (result == 0) {
        /* Process still running, wait for real */
        waitpid(pid, &status, 0);
    }

    return 1;
}

int main(int argc, char *argv[]) {
    if (argc != 4) {
        fprintf(stderr, "Usage: %s <pid> <lockfile> <lockfile.lock>\n", argv[0]);
        return 1;
    }

    pid_t target_pid = atoi(argv[1]);
    const char *lockfile = argv[2];
    const char *lockfile_lock = argv[3];

    if (target_pid <= 0) {
        fprintf(stderr, "Invalid PID: %s\n", argv[1]);
        return 1;
    }

    /* Detach from parent */
    setsid();

    /* Wait for target process to exit */
    if (!try_waitpid(target_pid)) {
        /* waitpid didn't work, fall back to polling */
        wait_polling(target_pid);
    }

    /* Target has exited - update lockfile */
    int lock_fd = open(lockfile_lock, O_WRONLY | O_CREAT, 0644);
    if (lock_fd == -1) {
        perror("Failed to open lock file");
        return 1;
    }

    /* Acquire exclusive lock */
    if (flock(lock_fd, LOCK_EX) == -1) {
        perror("Failed to acquire lock");
        close(lock_fd);
        return 1;
    }

    /* Check if lockfile exists */
    if (access(lockfile, F_OK) == 0) {
        int refcount = read_refcount(lockfile);

        if (refcount <= 1) {
            /* Last reference - remove lockfile */
            unlink(lockfile);
            unlink(lockfile_lock);
        } else if (refcount > 1) {
            /* Decrement refcount */
            write_refcount(lockfile, refcount - 1);
        }
    }

    /* Release lock and close */
    flock(lock_fd, LOCK_UN);
    close(lock_fd);

    return 0;
}
