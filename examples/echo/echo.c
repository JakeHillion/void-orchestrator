#include <netinet/in.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <unistd.h>

#define BUF_SIZE 1024

int connection_listener(int, int);
int request_handler(int);

int main(int argc, char **argv) {
  if (argc < 2) {
    fprintf(stderr, "usage: %s SUBCOMMAND...\n", argv[0]);
    return -1;
  }

  if (strcmp(argv[1], "connection_listener") == 0) {
    if (argc != 4) {
      fprintf(stderr, "usage: %s connection_listener SERVER_FD WRITE_PIPE_FD\n",
              argv[0]);
      return -1;
    }

    return connection_listener(atoi(argv[2]), atoi(argv[3]));
  } else if (strcmp(argv[1], "request_handler") == 0) {
    if (argc != 3) {
      fprintf(stderr, "usage: %s request_handler CLIENT_FD\n", argv[0]);
      return -1;
    }

    return request_handler(atoi(argv[2]));
  }

  fprintf(stderr, "unrecognised subcommand\n");
  return -1;
}

int connection_listener(int server_fd, int write_pipe) {
  while (1) {
    struct sockaddr_in client;
    socklen_t client_len = sizeof(client);

    int client_fd = accept(server_fd, (struct sockaddr *)&client, &client_len);
    if (client_fd < 0) {
      perror("accept");
    }

    write(write_pipe, &client_fd, sizeof(client_fd));
    fprintf(stderr, "sent client_fd: %d\n", client_fd);
  }

  return 0;
}

int request_handler(int client_fd) {
  char buf[BUF_SIZE];
  while (1) {
    int read = recv(client_fd, buf, BUF_SIZE, 0);
    if (read < 0) {
      perror("recv");
      return read;
    }

    if (!read) {
      fprintf(stderr, "connection terminated\n");
      return 0;
    }

    if (send(client_fd, buf, read, 0) < 0) {
      perror("send");
      return -1;
    }
  }

  return 0;
}
