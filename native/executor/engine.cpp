// Persistent planner process. Only the trusted parent publishes P-validated frames.
// Initialization never ticks; no ROS, driver, native I/O, plugin loading or network API.
#include "runtime.hpp"
#include <behaviortree_cpp/contrib/json.hpp>
#include <charconv>
#include <fstream>
#include <iostream>
#include <limits>
#include <regex>
using namespace rx::bt;
namespace {
using Json = nlohmann::json;
constexpr std::size_t max_packet = 4194304;
void require(bool value, const char* why) { if (!value) throw std::invalid_argument(why); }
void keys(const Json& value, std::initializer_list<const char*> names) {
  require(value.is_object() && value.size() == names.size(), "IPC field set differs");
  for (auto key : names) require(value.contains(key), "IPC field missing");
}
std::string text(const Json& value) { require(value.is_string(), "string required"); return value.get<std::string>(); }
std::string name(const Json& value) {
  const auto result = text(value); static const std::regex pattern("[A-Za-z0-9][A-Za-z0-9._/-]{0,127}");
  require(std::regex_match(result, pattern), "Name required"); return result;
}
std::string uuid(const Json& value) {
  const auto result = text(value); static const std::regex pattern("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}");
  require(std::regex_match(result, pattern), "UUID required"); return result;
}
std::string digest(const Json& value) {
  auto result = text(value); require(result.size() == 64 && result.find_first_not_of("0123456789abcdef") == std::string::npos, "digest required"); return result;
}
std::uint64_t counter(const Json& value) {
  const auto s = text(value); require(!s.empty() && (s.size() == 1 || s[0] != '0'), "decimal counter required");
  std::uint64_t result = 0; auto p = std::from_chars(s.data(), s.data()+s.size(), result);
  require(p.ec == std::errc{} && p.ptr == s.data()+s.size(), "uint64 counter required"); return result;
}
Json parse(const std::string& packet) {
  std::vector<std::set<std::string>> objects;
  return Json::parse(packet, [&](int depth, Json::parse_event_t event, Json& value) {
    require(depth <= 64, "IPC nesting limit");
    if (event == Json::parse_event_t::object_start) objects.emplace_back();
    if (event == Json::parse_event_t::key) require(!objects.empty() && objects.back().insert(value.get<std::string>()).second, "duplicate JSON key");
    if (event == Json::parse_event_t::object_end) objects.pop_back();
    return true;
  });
}
bool read_packet(std::string& packet) {
  packet.clear(); char c;
  while (std::cin.get(c)) {
    if (c == '\n') return true;
    require(packet.size() < max_packet, "IPC packet limit"); packet.push_back(c);
  }
  require(packet.empty(), "truncated IPC packet"); return false;
}
Identity identity(const Json& value) {
  keys(value, {"run","executor_session","resolved_digest","visit","epoch"});
  Identity result{uuid(value.at("run")),uuid(value.at("executor_session")),digest(value.at("resolved_digest")),counter(value.at("visit")),counter(value.at("epoch"))};
  require(result.visit > 0 && result.epoch > 0, "positive context counters required"); return result;
}
std::pair<std::string, std::map<std::string, Definition>> definitions(const Json& process) {
  keys(process, {"schema","package_digest","source_digest","process","root","bindings","conditions"});
  require(text(process.at("schema")) == "rx.resolved-process.v1", "resolved schema differs");
  require(process.dump().size() <= 1000000, "resolved payload limit");
  std::map<std::string, Definition> result;
  std::function<void(const Json&, int)> collect = [&](const Json& node, int depth) {
    require(depth <= 64 && result.size() < 4096, "resolved node limit"); keys(node,{"id","source","body"});
    const auto id = name(node.at("id")); const auto& body=node.at("body"); const auto kind=text(body.at("kind")); Definition def{};
    if (kind=="SEQUENCE" || kind=="PARALLEL_ALL") {
      keys(body,{"kind","children"}); require(body.at("children").is_array(),"children required");
      def.kind=kind=="SEQUENCE"?Kind::Sequence:Kind::ParallelAll;
      for(const auto& child:body.at("children")){def.children.push_back(name(child.at("id")));collect(child,depth+1);}
    } else if(kind=="BRANCH") {
      keys(body,{"kind","condition","when_true","when_false"}); def.kind=Kind::Branch;def.argument=name(body.at("condition"));
      for(auto key:{"when_true","when_false"}){const auto& child=body.at(key);def.children.push_back(name(child.at("id")));collect(child,depth+1);}
    } else if(kind=="OPERATION") {keys(body,{"kind","binding"});def.kind=Kind::Operation;def.argument=name(body.at("binding"));}
    else if(kind=="WAIT") {keys(body,{"kind","condition","timeout_ns"});def.kind=Kind::Wait;def.argument=name(body.at("condition"));def.timeout_ns=counter(body.at("timeout_ns"));require(def.timeout_ns>0,"wait timeout required");}
    else if(kind=="INTERVENTION") {keys(body,{"kind","procedure"});keys(body.at("procedure"),{"sha256","schema_id","size_bytes"});def.kind=Kind::Intervention;def.argument=digest(body.at("procedure").at("sha256"));}
    else throw std::invalid_argument("unsupported resolved node kind");
    require(result.emplace(id,std::move(def)).second,"duplicate resolved node");
  };
  collect(process.at("root"),0); return {name(process.at("root").at("id")),std::move(result)};
}
const char* kind(RequestKind k) {
  switch(k){case RequestKind::SubmitOperation:return "SUBMIT_OPERATION";case RequestKind::ResolveBranch:return "RESOLVE_BRANCH";case RequestKind::BeginWait:return "BEGIN_WAIT";case RequestKind::RequestIntervention:return "REQUEST_INTERVENTION";case RequestKind::RequestHandover:return "REQUEST_HANDOVER";case RequestKind::PauseExecutor:return "PAUSE_EXECUTOR";}
  throw std::logic_error("request kind");
}
Json requests(const std::shared_ptr<Context>& context) {
  auto result=Json::array(); if(!context)return result;
  for(const auto& request:context->take_requests()) { const auto& i=request.identity;
    result.push_back({{"identity",{{"run",i.run},{"executor_session",i.executor_session},{"resolved_digest",i.resolved_digest},{"visit",std::to_string(i.visit)},{"epoch",std::to_string(i.epoch)}}},{"node",request.node},{"kind",kind(request.kind)},{"argument",request.argument},{"timeout_ns",std::to_string(request.timeout_ns)}});
  }
  return result;
}
void reply(std::uint64_t sequence, const char* state, const std::shared_ptr<Context>& context, Json fault=nullptr) {
  std::cout << Json({{"schema","rx.bt-reply.v1"},{"sequence",std::to_string(sequence)},{"state",state},{"requests",requests(context)},{"fault",fault}}).dump(-1,' ',false,Json::error_handler_t::replace) << '\n' << std::flush;
  require(std::cout.good(),"IPC output closed");
}
#ifdef RX_BT_TEST_CLOCK
class TestClock final : public FrameClock {
 public: explicit TestClock(std::string path):path_(std::move(path)){}
  ClockSample now() const override {std::ifstream input(path_);std::string value((std::istreambuf_iterator<char>(input)),{});require(value.size()<=256,"test clock limit");return {"test-clock",counter(parse(value))};}
 private: std::string path_;
};
#endif
}
int main(int argc,char** argv) {
  std::shared_ptr<Context> context; std::unique_ptr<Executor> executor; std::uint64_t sequence=0, revision=0; bool halted=false; Identity expected{};
  try {
#ifdef RX_BT_TEST_CLOCK
    require(argc==2,"test clock file required"); auto clock=std::make_shared<TestClock>(argv[1]);
#else
    (void)argv; require(argc==1,"production engine accepts no clock override"); auto clock=linux_boottime_clock();
#endif
    std::string packet;
    while(read_packet(packet)) {
      auto value=parse(packet); require(text(value.at("schema"))=="rx.bt-command.v1","IPC schema differs");
      require(sequence<std::numeric_limits<std::uint64_t>::max() && counter(value.at("sequence"))==sequence+1,"IPC sequence differs"); ++sequence;
      const auto command=text(value.at("command"));
      if(command=="INITIALIZE") {
        keys(value,{"schema","sequence","command","identity","resolved","xml"});require(!executor,"already initialized");
        auto [root, defs]=definitions(value.at("resolved"));auto xml=text(value.at("xml"));require(xml.size()<=1000000,"XML limit");
        expected=identity(value.at("identity"));context=std::make_shared<Context>(expected,std::move(root),std::move(defs),clock);
        executor=std::make_unique<Executor>(std::move(xml),context);reply(sequence,"READY",context);
      } else if(command=="STEP") {
        keys(value,{"schema","sequence","command","frame"});require(executor && !halted,"engine not running");
        auto frame=decode_frame(value.at("frame").dump());require(frame.identity==expected && frame.revision>=revision,"frame context/position differs");
        const auto now=clock->now();const auto& bound=*frame.source_deadline;
        require(now.clock_id==bound.clock_id && now.ticks_ns>=bound.checked_at_ns,"frame clock continuity differs");
        if(now.ticks_ns>=bound.valid_until_ns){reply(sequence,"STALE",context);continue;}
        revision=frame.revision;context->publish(std::move(frame));const auto result=executor->tick();
        const char* state=result==BT::NodeStatus::RUNNING?"RUNNING":result==BT::NodeStatus::SUCCESS?"SUCCESS":result==BT::NodeStatus::FAILURE?"FAILURE":nullptr;
        require(state,"unsupported BT status");reply(sequence,state,context);
      } else if(command=="HALT" || command=="CLOSE") {
        keys(value,{"schema","sequence","command"});require(executor!=nullptr,"engine not initialized");halted=true;executor->halt();reply(sequence,"HALTED",context);
        if(command=="CLOSE")return 0;
      } else throw std::invalid_argument("unsupported IPC command");
    }
    if(executor)executor->halt(); // Parent loss never keeps a planner running.
    return 0;
  } catch(const std::exception& error) {
    if(executor)executor->halt();
    try{reply(sequence,"FAULT",context,std::string(error.what()).substr(0,1024));}catch(...){}
    return 2;
  }
}
