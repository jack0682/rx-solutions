// Validation-only one-frame request producer. No device or network interfaces.
#include "runtime.hpp"
#include <behaviortree_cpp/contrib/json.hpp>
#include <fstream>
#include <iostream>
using namespace rx::bt;
namespace {
class Clock final : public FrameClock {
 public: ClockSample value;
  ClockSample now() const override {return value;}
};
std::string load(const char* path) {std::ifstream input(path);return std::string((std::istreambuf_iterator<char>(input)),{});}
const char* kind(RequestKind value) {
  switch(value) {
    case RequestKind::SubmitOperation:return "SUBMIT_OPERATION";
    case RequestKind::ResolveBranch:return "RESOLVE_BRANCH";
    case RequestKind::BeginWait:return "BEGIN_WAIT";
    case RequestKind::RequestIntervention:return "REQUEST_INTERVENTION";
    case RequestKind::RequestHandover:return "REQUEST_HANDOVER";
    case RequestKind::PauseExecutor:return "PAUSE_EXECUTOR";
  }
  throw std::runtime_error("unknown request kind");
}
}
int main(int argc,char** argv) {
  try {
    if(argc!=6 && argc!=7) throw std::runtime_error("expected resolved, XML, frame, test clock ID and ticks");
    std::string clock_id=argv[4];if(clock_id!="test-clock" && clock_id.rfind("simulation/",0)!=0 && clock_id.rfind("test/",0)!=0) throw std::runtime_error("explicit simulation clock required");
    auto process=nlohmann::json::parse(load(argv[1]));std::map<std::string,Definition> definitions;
    std::function<void(const nlohmann::json&,int)> collect=[&](const auto& node,int depth) {
      if(depth>64 || definitions.size()>=4096) throw std::runtime_error("definition limit");
      auto body=node.at("body");std::string type=body.at("kind");Definition def{};
      if(type=="SEQUENCE" || type=="PARALLEL_ALL") {def.kind=type=="SEQUENCE"?Kind::Sequence:Kind::ParallelAll;for(const auto& child:body.at("children")){def.children.push_back(child.at("id"));collect(child,depth+1);}}
      else if(type=="BRANCH") {def.kind=Kind::Branch;def.argument=body.at("condition");for(auto key:{"when_true","when_false"}){const auto& child=body.at(key);def.children.push_back(child.at("id"));collect(child,depth+1);}}
      else if(type=="OPERATION") {def.kind=Kind::Operation;def.argument=body.at("binding");}
      else if(type=="WAIT") {def.kind=Kind::Wait;def.argument=body.at("condition");def.timeout_ns=std::stoull(body.at("timeout_ns").template get<std::string>());}
      else if(type=="INTERVENTION") {def.kind=Kind::Intervention;def.argument=body.at("procedure").at("sha256");}
      else throw std::runtime_error("unknown node");
      if(!definitions.emplace(node.at("id").template get<std::string>(),std::move(def)).second) throw std::runtime_error("duplicate node");
    };
    collect(process.at("root"),0);auto frame=decode_frame(load(argv[3]));auto clock=std::make_shared<Clock>();clock->value={clock_id,std::stoull(argv[5])};
    auto context=std::make_shared<Context>(frame.identity,process.at("root").at("id"),definitions,clock);
    Executor executor(load(argv[2]),context);context->publish(frame);executor.tick();if(argc==7){if(std::string(argv[6])!="--halt")throw std::runtime_error("unknown fixture mode");executor.halt();}auto requests=context->take_requests();auto output=nlohmann::json::array();
    for(const auto& request:requests) {const auto& i=request.identity;output.push_back({{"identity",{{"run",i.run},{"executor_session",i.executor_session},{"resolved_digest",i.resolved_digest},{"visit",std::to_string(i.visit)},{"epoch",std::to_string(i.epoch)}}},
      {"node",request.node},{"kind",kind(request.kind)},{"argument",request.argument},{"timeout_ns",std::to_string(request.timeout_ns)}});}
    std::cout<<output.dump()<<'\n';return 0;
  } catch(const std::exception& e) {std::cerr<<e.what()<<'\n';return 1;}
}
