// Negative caller only: never installed in the product or client library.
#include <sys/socket.h>
#include <sys/wait.h>
#include <unistd.h>
#include <cstring>
int main(int argc,char**argv){
  if(argc!=3)return 90;
  int channel[2]{};const bool direct=std::strcmp(argv[2],"direct")==0;
  if(!direct && socketpair(AF_UNIX,SOCK_STREAM|SOCK_CLOEXEC,0,channel))return 91;
  const auto pid=fork();
  if(pid==0){
    if(!direct){dup2(channel[1],0);close(channel[0]);close(channel[1]);}
    execl(argv[1],std::strcmp(argv[2],"spoof")==0?"/opt/rx/bin/rx-hostd":argv[1],"--endpoint","simulation/dynamixel/id-1",nullptr);_exit(92);
  }
  if(!direct){
    close(channel[1]);
    const char request[]=R"({"operation":"11111111-1111-4111-8111-111111111111","invocation":"22222222-2222-4222-8222-222222222222","instance":"33333333-3333-4333-8333-333333333333","deadline_ns":"18446744073709551615"})" "\n";
    if(write(channel[0],request,sizeof(request)-1)<0)return 95;
  }
  int status;if(waitpid(pid,&status,0)!=pid)return 93;
  if(!direct)close(channel[0]);
  return WIFEXITED(status)?WEXITSTATUS(status):94;
}
