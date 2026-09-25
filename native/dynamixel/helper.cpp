// A single read-only simulated transport. No OS serial PortHandler is linked.
#include "protocol2_packet_handler.h"
#include <nlohmann/json.hpp>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/prctl.h>
#include <unistd.h>
#include <signal.h>
#include <time.h>
#include <array>
#include <algorithm>
#include <cstdio>
#include <cstring>
#include <stdexcept>
#include <string>
#include <vector>
#include <thread>
#include <chrono>
using nlohmann::json;
namespace {
constexpr auto endpoint = "simulation/dynamixel/id-1";
[[noreturn]] void refuse(const char* reason) { throw std::runtime_error(reason); }
std::string hex(const std::vector<uint8_t>& data) {
  constexpr char digits[] = "0123456789abcdef"; std::string out;
  for(auto b:data){out+=digits[b>>4];out+=digits[b&15];} return out;
}
uint16_t crc(const std::vector<uint8_t>& data) {
  uint16_t value=0;
  for(auto b:data){value^=uint16_t(b)<<8;for(int i=0;i<8;++i)value=(value&0x8000)?uint16_t((value<<1)^0x8005):uint16_t(value<<1);}
  return value;
}
class SimPort final : public dynamixel::PortHandler {
  size_t cursor=0;
public:
  std::vector<uint8_t> tx,rx; unsigned writes=0;
  SimPort(){is_using_=false;}
  bool openPort() override{return true;}
  void closePort() override{}
  void clearPort() override{cursor=0;}
  void setPortName(const char*) override{refuse("DXL_ENDPOINT_FIXED");}
  char* getPortName() override{static char value[]="simulation/dynamixel/id-1";return value;}
  bool setBaudRate(int) override{return false;}
  int getBaudRate() override{return DEFAULT_BAUDRATE_;}
  int getBytesAvailable() override{return int(rx.size()-cursor);}
  int readPort(uint8_t* out,int count) override{
    const auto n=std::min(size_t(count),rx.size()-cursor);std::copy_n(rx.data()+cursor,n,out);cursor+=n;return int(n);
  }
  int writePort(uint8_t* data,int count) override{
    tx.assign(data,data+count);++writes;
    if(writes!=1 || hex(tx)!="fffffd0001030001194e")refuse("DXL_PING_ONLY");
    // Fictional RX model 65500, firmware 1; not an actual product model claim.
    rx={0xff,0xff,0xfd,0,1,7,0,0x55,0,0xdc,0xff,1};auto c=crc(rx);rx.push_back(c&255);rx.push_back(c>>8);cursor=0;
    // Fixed modeled response latency, no configurable fault switch.
    std::this_thread::sleep_for(std::chrono::milliseconds(250));return count;
  }
  void setPacketTimeout(uint16_t) override{}
  void setPacketTimeout(double) override{}
  bool isPacketTimeout() override{return cursor>=rx.size();}
};
void host_channel(){
  const auto parent=getppid();ucred peer{};socklen_t length=sizeof(peer);struct stat actual{},expected{};
  // No listener exists. Only the inherited channel addresses this instance.
  // Parent identity is independently required to reject a second callable helper.
  // Both checks are load-bearing; neither creates a same-UID sandbox.
  if(prctl(PR_SET_PDEATHSIG,SIGKILL)||getppid()!=parent || getsockopt(0,SOL_SOCKET,SO_PEERCRED,&peer,&length)||peer.pid!=parent)
    refuse("DXL_HOST_CHANNEL_REQUIRED");
  auto path=std::string("/proc/")+std::to_string(parent)+"/exe";
  if(stat(path.c_str(),&actual)||stat("/opt/rx/bin/rx-hostd",&expected)||actual.st_dev!=expected.st_dev||actual.st_ino!=expected.st_ino)
    refuse("DXL_HOST_EXECUTABLE_REQUIRED");
}
uint64_t now(){timespec t{};if(clock_gettime(CLOCK_BOOTTIME,&t))refuse("DXL_CLOCK_UNAVAILABLE");return uint64_t(t.tv_sec)*1000000000+uint64_t(t.tv_nsec);}
}
int main(int argc,char**argv){
  try{
    if(argc!=3 || std::string(argv[1])!="--endpoint")refuse("DXL_FIXED_ENDPOINT_REQUIRED");
    if(std::string(argv[2])!=endpoint)refuse("DXL_REAL_ENDPOINT_UNSUPPORTED");
    host_channel();std::string line;char c=0;
    while(line.size()<2048 && read(0,&c,1)==1 && c!='\n')line+=c;
    if(c!='\n' || line.size()>=2048)refuse("DXL_HOST_REQUEST_REQUIRED");
    const auto request=json::parse(line);
    const std::array<std::string,4> keys={"operation","invocation","instance","deadline_ns"};
    if(!request.is_object()||request.size()!=keys.size())refuse("DXL_REQUEST_SHAPE");
    for(const auto&k:keys)if(!request.contains(k)||!request[k].is_string())refuse("DXL_REQUEST_SHAPE");
    for(const auto&k:{"operation","invocation","instance"})if(request[k].get<std::string>().size()!=36)refuse("DXL_ID_SHAPE");
    const auto deadline=std::stoull(request["deadline_ns"].get<std::string>());
    if(now()>=deadline)refuse("DXL_DISPATCH_EXPIRED");
    auto audit=request;audit["event"]="helper_entered";audit["pid"]=getpid();audit["parent_pid"]=getppid();
    const auto raw=audit.dump()+"\n";if(write(2,raw.data(),raw.size())!=ssize_t(raw.size())||fsync(2))refuse("DXL_AUDIT_UNAVAILABLE");
    SimPort port;uint16_t model=0;uint8_t error=0;
    const auto result=dynamixel::Protocol2PacketHandler::getInstance()->ping(&port,1,&model,&error);
    if(result!=COMM_SUCCESS || error || model!=65500 || port.writes!=1)refuse("DXL_PING_UNCONFIRMED");
    const auto response=json{{"schema","rx.dynamixel.ping-capture.v1"},{"request",request},{"sdk","4.1.0"},{"transport","SIMULATED"},{"model",model},{"protocol",2},{"device_id",1},{"result",result},{"error",error},{"writes",port.writes},{"tx",hex(port.tx)},{"rx",hex(port.rx)}}.dump()+"\n";
    if(send(0,response.data(),response.size(),MSG_NOSIGNAL)!=ssize_t(response.size()))refuse("DXL_REPLY_LOST");
    return 0;
  }catch(const std::exception&e){std::fprintf(stderr,"%s\n",e.what());return 21;}
}
