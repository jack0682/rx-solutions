#include "runtime.hpp"
#include <behaviortree_cpp/contrib/json.hpp>
#include <fstream>
#include <iostream>
#include <stdexcept>

using namespace rx::bt;
namespace {
void require(bool value, const std::string& message) { if (!value) throw std::runtime_error(message); }
template <typename F> void rejects(F fn, const char* message) {
  try { fn(); } catch (const std::exception&) { return; }
  throw std::runtime_error(message);
}
Identity identity() { return {"run-1", "session-1", std::string(64, 'a'), 1, 1}; }
Frame frame() { Frame result; result.identity = identity(); result.revision = 1; result.complete = true; result.current = true; result.submission_authorized = true; result.valid_until = std::chrono::steady_clock::now() + std::chrono::hours(1); return result; }
std::string xml(const std::string& body) { return "<root BTCPP_format=\"4\" main_tree_to_execute=\"RXMain\"><BehaviorTree ID=\"RXMain\">" + body + "</BehaviorTree></root>"; }
Observation operation(std::string id, Outcome outcome = Outcome::None, bool released = false) {
  Observation result; result.operation_id = std::move(id); result.outcome = outcome; result.released = released; return result;
}
void sequence_and_handover() {
  auto context = std::make_shared<Context>(identity(), "root", std::map<std::string, Definition>{
      {"root", {Kind::Sequence, {"a", "b"}, {}, 0}}, {"a", {Kind::Operation, {}, "load", 0}}, {"b", {Kind::Operation, {}, "close", 0}}});
  auto text = xml("<RXSequence node_id=\"root\"><RXOperation node_id=\"a\" binding=\"load\"/><RXOperation node_id=\"b\" binding=\"close\"/></RXSequence>");
  Executor executor(text, context); auto state = frame(); state.eligible = {"a", "b"}; context->publish(state);
  for (int i = 0; i < 10; ++i) require(executor.tick() == BT::NodeStatus::RUNNING, "operation should wait");
  auto requests = context->take_requests(); require(requests.size() == 1 && requests[0].node == "a", "repeated ticks duplicated admission");
  state.revision++; state.nodes["a"] = operation("op-a", Outcome::Succeeded); context->publish(state);
  require(executor.tick() == BT::NodeStatus::RUNNING, "success is not handover"); requests = context->take_requests();
  require(requests.size() == 1 && requests[0].kind == RequestKind::RequestHandover, "handover request missing");
  state.revision++; state.nodes["a"].released = true; context->publish(state); executor.tick(); requests = context->take_requests();
  require(requests.size() == 1 && requests[0].node == "b", "next sequence step not admitted");
  state.revision++; state.nodes["b"] = operation("op-b"); state.nodes["b"].unknown = true; context->publish(state);
  require(executor.tick() == BT::NodeStatus::RUNNING, "unknown became terminal failure"); require(context->take_requests().empty(), "unknown retried native work");
  executor.halt(); requests = context->take_requests(); require(requests.size() == 1 && requests[0].kind == RequestKind::PauseExecutor, "halt should request pause only");
  Executor rebuilt(text, context); rebuilt.tick(); require(context->take_requests().empty(), "rebuilding tree replayed an existing request");
}
void decisions_waits_and_interventions() {
  auto context = std::make_shared<Context>(identity(), "branch", std::map<std::string, Definition>{
      {"branch", {Kind::Branch, {"wait", "access"}, "ready", 0}}, {"wait", {Kind::Wait, {}, "ready", 5000}}, {"access", {Kind::Intervention, {}, std::string(64, 'b'), 0}}});
  Executor executor(xml("<RXBranch node_id=\"branch\" condition_id=\"ready\"><RXWait node_id=\"wait\" condition_id=\"ready\" timeout_ns=\"5000\"/><RXIntervention node_id=\"access\" procedure_digest=\"" + std::string(64, 'b') + "\"/></RXBranch>"), context);
  auto state = frame(); state.eligible = {"branch", "wait", "access"}; context->publish(state); executor.tick(); auto requests = context->take_requests();
  require(requests.size() == 1 && requests[0].kind == RequestKind::ResolveBranch, "branch guessed without P decision");
  state.revision++; state.nodes["branch"].branch = true; state.nodes["branch"].decision_id = "decision-1"; context->publish(state);
  for (int i = 0; i < 20; ++i) require(executor.tick() == BT::NodeStatus::RUNNING, "wait used local timeout/completion");
  requests = context->take_requests(); require(requests.size() == 1 && requests[0].kind == RequestKind::BeginWait, "wait duplicated or took other branch");
  auto changed = state; changed.revision++; changed.nodes["branch"].branch = false;
  rejects([&] { context->publish(changed); }, "branch choice changed after commitment");
  state.revision++; state.nodes["wait"].wait = WaitState::Satisfied; state.nodes["wait"].decision_id = "wait-decision-1"; context->publish(state); require(executor.tick() == BT::NodeStatus::SUCCESS, "recorded wait result was not used");
  auto access_context = std::make_shared<Context>(identity(), "access", std::map<std::string, Definition>{{"access", {Kind::Intervention, {}, std::string(64, 'b'), 0}}});
  Executor access(xml("<RXIntervention node_id=\"access\" procedure_digest=\"" + std::string(64, 'b') + "\"/>"), access_context);
  auto access_state = frame(); access_state.eligible = {"access"}; access_context->publish(access_state); access.tick();
  require(access_context->take_requests().at(0).kind == RequestKind::RequestIntervention, "intervention request missing");
  access_state.revision++; access_state.nodes["access"].clearance = "clearance-1"; access_context->publish(access_state); require(access.tick() == BT::NodeStatus::SUCCESS, "clearance not observed");
}
void unknown_parallel_and_revocation() {
  auto context = std::make_shared<Context>(identity(), "root", std::map<std::string, Definition>{
      {"root", {Kind::ParallelAll, {"a", "b"}, {}, 0}}, {"a", {Kind::Operation, {}, "a", 0}}, {"b", {Kind::Operation, {}, "b", 0}}});
  Executor executor(xml("<RXParallelAll node_id=\"root\"><RXOperation node_id=\"a\" binding=\"a\"/><RXOperation node_id=\"b\" binding=\"b\"/></RXParallelAll>"), context);
  auto state = frame(); state.eligible = {"a", "b"}; state.nodes["a"] = operation("op-a"); state.nodes["a"].unknown = true; context->publish(state);
  require(executor.tick() == BT::NodeStatus::RUNNING, "parallel unknown failed"); require(context->take_requests().empty(), "unknown parallel admitted new sibling");
  state.revision++; state.nodes["a"].unknown = false; state.nodes["a"].outcome = Outcome::Succeeded; state.nodes["a"].released = true; context->publish(state); executor.tick();
  state.revision++; state.submission_authorized = false; context->publish(state);
  require(context->take_requests().empty(), "queued admission escaped after revocation");
  auto broken = state; broken.revision++; broken.nodes["a"].operation_id = "different";
  rejects([&] { context->publish(broken); }, "operation identity was rebound");
  auto partial = state; partial.revision++; partial.complete = false; context->publish(partial); require(executor.tick() == BT::NodeStatus::RUNNING, "partial frame drove progress");
  auto paused = context->take_requests(); require(paused.size() == 1 && paused[0].kind == RequestKind::PauseExecutor, "invalid frame did not latch pause");
  auto failed_context = std::make_shared<Context>(identity(), "root", context->definitions());
  Executor failed(xml("<RXParallelAll node_id=\"root\"><RXOperation node_id=\"a\" binding=\"a\"/><RXOperation node_id=\"b\" binding=\"b\"/></RXParallelAll>"), failed_context);
  auto failure = frame(); failure.eligible = {"a", "b"}; failure.nodes["a"] = operation("failed-a", Outcome::Failed); failed_context->publish(failure);
  require(failed.tick() == BT::NodeStatus::FAILURE && failed_context->take_requests().empty(), "failed parallel launched an unstarted sibling");
}
void xml_whitelist_and_queue_bound() {
  auto context = std::make_shared<Context>(identity(), "a", std::map<std::string, Definition>{{"a", {Kind::Operation, {}, "load", 0}}});
  rejects([&] { Executor unsafe(xml("<RXOperation node_id=\"a\" binding=\"load\" _onSuccess=\"x:=1\"/>"), context); }, "XML script accepted");
  rejects([&] { Executor unsafe(xml("<RXOperation node_id=\"a\" binding=\"other\"/>"), context); }, "XML binding changed");
  rejects([&] { Executor unsafe(xml("<AlwaysSuccess/>"), context); }, "unregistered control escape accepted");
  Executor guarded(xml("<RXOperation node_id=\"a\" binding=\"load\"/>"), context);
  auto expired = frame(); expired.eligible = {"a"}; expired.valid_until = std::chrono::steady_clock::now(); context->publish(expired);
  require(guarded.tick() == BT::NodeStatus::RUNNING && context->take_requests().empty(), "expired view admitted a request");
  std::map<std::string, Definition> definitions; auto state = frame();
  for (int i = 0; i < 40; ++i) { auto id = std::to_string(i); definitions.emplace(id, Definition{Kind::Operation, {}, id, 0}); state.eligible.insert(id); }
  auto bounded = std::make_shared<Context>(identity(), "0", definitions); bounded->publish(state); bounded->begin_tick();
  int accepted = 0;
  for (int i = 0; i < 40; ++i) accepted += bounded->enqueue(std::to_string(i), RequestKind::SubmitOperation) ? 1 : 0;
  require(accepted == 32, "normal request capacity differs");
  bounded->pause("0"); auto requests = bounded->take_requests(); require(requests.size() == 1 && requests[0].kind == RequestKind::PauseExecutor, "reserved pause or revoked queue violated");
}

class ManualFrameClock : public FrameClock {
 public:
  std::uint64_t ticks = 1000;
  std::string id = "test/boottime";
  ClockSample now() const override { return {id, ticks}; }
};
nlohmann::json packet() {
  return {{"schema","rx.bt-frame.v1"},{"identity",{{"run","00000000-0000-4000-8000-000000000001"},{"executor_session","00000000-0000-4000-8000-000000000002"},{"resolved_digest",std::string(64,'a')},{"visit","1"},{"epoch","1"}}},
    {"revision","1"},{"complete",true},{"current",true},{"submission_authorized",true},{"remaining_validity_ns","100000000"},
    {"source",{{"clock_id","test/boottime"},{"checked_at_ns","1000"},{"valid_until_ns","100001000"}}},{"nodes",nlohmann::json::object()},{"eligible",{"a"}}};
}
void source_clock_and_frame_parser() {
  auto clock = std::make_shared<ManualFrameClock>(); auto source = packet(); auto state = decode_frame(source.dump());
  auto context = std::make_shared<Context>(state.identity,"a",std::map<std::string,Definition>{{"a",{Kind::Operation,{},"load",0}}},clock);
  Executor executor(xml("<RXOperation node_id=\"a\" binding=\"load\"/>"),context);
  context->publish(state); executor.tick();
  clock->ticks = 100001000;
  require(context->take_requests().empty(),"source clock expiry did not stop queued request");
  source["revision"]="2"; source["source"]["checked_at_ns"]="100001000"; source["source"]["valid_until_ns"]="200001000";
  context->publish(decode_frame(source.dump())); executor.tick();
  auto requests = context->take_requests(); require(requests.size()==1 && requests[0].kind==RequestKind::SubmitOperation,"fresh view lost or duplicated retained request");
  executor.tick(); require(context->take_requests().empty(),"request replayed after handoff");
  clock->id = "another/boot";
  rejects([&] { context->publish(decode_frame(source.dump())); },"foreign source clock accepted");
  auto malformed = packet(); malformed["identity"]["visit"] = 1;
  rejects([&] { decode_frame(malformed.dump()); },"numeric uint64 accepted");
  malformed = packet(); malformed["revision"]="18446744073709551616";
  rejects([&] { decode_frame(malformed.dump()); },"uint64 overflow accepted");
  malformed = packet(); malformed["unknown_field"]=true;
  rejects([&] { decode_frame(malformed.dump()); },"extra frame field accepted");
  malformed = packet(); malformed["eligible"].push_back("a");
  rejects([&] { decode_frame(malformed.dump()); },"duplicate eligible entry accepted");
  auto duplicate = packet().dump(); duplicate.insert(1,"\"complete\":true,");
  rejects([&] { decode_frame(duplicate); },"duplicate JSON field accepted");
  auto no_clock = std::make_shared<Context>(state.identity,"a",context->definitions());
  rejects([&] { no_clock->publish(decode_frame(packet().dump())); },"source-bound frame accepted without clock adapter");
#ifdef __linux__
  auto native = linux_boottime_clock(); auto first = native->now(); auto next = native->now();
  require(first.clock_id.rfind("linux-boottime/",0)==0 && next.clock_id==first.clock_id && next.ticks_ns>=first.ticks_ns,"Linux boottime identity/monotonicity");
#endif
}

void compiled_example(const std::string& resolved_path, const std::string& xml_path) {
  nlohmann::json process; std::ifstream source(resolved_path); source >> process;
  std::map<std::string, Definition> definitions;
  std::function<void(const nlohmann::json&)> collect = [&](const auto& node) {
    const auto& body = node.at("body"); const auto kind = body.at("kind").template get<std::string>(); Definition def{};
    if (kind == "SEQUENCE") { def.kind = Kind::Sequence; for (const auto& child : body.at("children")) { def.children.push_back(child.at("id")); collect(child); } }
    else if (kind == "OPERATION") { def.kind = Kind::Operation; def.argument = body.at("binding"); }
    else throw std::runtime_error("unexpected example node");
    definitions.emplace(node.at("id").template get<std::string>(), def);
  };
  collect(process.at("root")); auto context = std::make_shared<Context>(identity(), process.at("root").at("id"), definitions);
  std::ifstream input(xml_path); std::string text((std::istreambuf_iterator<char>(input)), {}); Executor executor(text, context);
  auto state = frame(); for (const auto& [id, def] : definitions) if (def.kind == Kind::Operation) state.eligible.insert(id);
  context->publish(state); std::size_t admitted = 0;
  while (executor.tick() != BT::NodeStatus::SUCCESS) {
    auto requests = context->take_requests(); require(requests.size() == 1 && requests[0].kind == RequestKind::SubmitOperation, "compiled sequence was not serialized");
    ++admitted; state.revision++; state.nodes[requests[0].node] = operation("op-" + std::to_string(admitted), Outcome::Succeeded, true); context->publish(state);
    require(admitted <= 5, "compiled example repeated work");
  }
  require(admitted == 5, "compiled example missed a step");
}

void persistent_wait_and_obsolete_queue() {
  auto context=std::make_shared<Context>(identity(), "wait", std::map<std::string,Definition>{{"wait",{Kind::Wait,{},"ready",5000}}});
  Executor executor(xml("<RXWait node_id=\"wait\" condition_id=\"ready\" timeout_ns=\"5000\"/>"),context);
  auto state=frame();state.eligible={"wait"};context->publish(state);executor.tick();
  state.revision++;state.nodes["wait"].wait=WaitState::Satisfied;state.nodes["wait"].decision_id="wait-final";
  context->publish(state);require(context->take_requests().empty(),"resolved undelivered wait kept occupying queue");
  require(executor.tick()==BT::NodeStatus::SUCCESS,"wait result not consumed");
  auto changed=state;changed.revision++;changed.nodes["wait"].wait=WaitState::TimedOut;changed.nodes["wait"].decision_id="different";
  rejects([&]{context->publish(changed);},"terminal wait decision changed");
  const auto pause=context->take_requests();require(pause.size()==1 && pause[0].kind==RequestKind::PauseExecutor,"wait contradiction did not request pause");
  std::map<std::string,Definition> defs;std::vector<std::string> children;std::string body="<RXParallelAll node_id=\"root\">";
  for(int i=0;i<40;++i){auto node="node/"+std::to_string(i);children.push_back(node);defs.emplace(node,Definition{Kind::Operation,{},"load",0});body+="<RXOperation node_id=\""+node+"\" binding=\"load\"/>";}
  body+="</RXParallelAll>";defs.emplace("root",Definition{Kind::ParallelAll,children,{},0});
  auto many=std::make_shared<Context>(identity(),"root",defs);Executor parallel(xml(body),many);auto current=frame();current.eligible.insert(children.begin(),children.end());many->publish(current);parallel.tick();
  for(int i=0;i<32;++i)current.nodes[children[i]]=operation("operation/"+std::to_string(i));
  current.revision++;many->publish(current);parallel.tick();auto next=many->take_requests();require(next.size()==8,"obsolete suggestions exhausted the request queue");
}

void restored_platform_frame(const std::string& resolved_path, const std::string& xml_path, const std::string& frame_path) {
  nlohmann::json process; std::ifstream source(resolved_path); source >> process;
  std::map<std::string, Definition> definitions;
  std::function<void(const nlohmann::json&, int)> collect = [&](const auto& node, int depth) {
    require(depth<=64 && definitions.size()<4096,"restored definition limit");
    const auto& body=node.at("body"); const auto kind=body.at("kind").template get<std::string>(); Definition def{};
    if (kind=="SEQUENCE" || kind=="PARALLEL_ALL") { def.kind=kind=="SEQUENCE"?Kind::Sequence:Kind::ParallelAll;
      for (const auto& child:body.at("children")) {def.children.push_back(child.at("id"));collect(child,depth+1);} }
    else if (kind=="BRANCH") {def.kind=Kind::Branch;def.argument=body.at("condition");
      for (const auto* field:{"when_true","when_false"}) {const auto& child=body.at(field);def.children.push_back(child.at("id"));collect(child,depth+1);} }
    else if (kind=="OPERATION") {def.kind=Kind::Operation;def.argument=body.at("binding");}
    else if (kind=="WAIT") {def.kind=Kind::Wait;def.argument=body.at("condition");def.timeout_ns=std::stoull(body.at("timeout_ns").template get<std::string>());}
    else if (kind=="INTERVENTION") {def.kind=Kind::Intervention;def.argument=body.at("procedure").at("sha256");}
    else throw std::runtime_error("unsupported restored node");
    require(definitions.emplace(node.at("id").template get<std::string>(),std::move(def)).second,"duplicate restored node");
  };
  collect(process.at("root"),0);
  std::ifstream frame_input(frame_path); std::string packet_text((std::istreambuf_iterator<char>(frame_input)),{});
  auto state=decode_frame(packet_text);
  require(!state.submission_authorized,"restart fixture unexpectedly restored admission authority");
  auto clock=std::make_shared<ManualFrameClock>();clock->id="test-clock";clock->ticks=1000;
  auto context=std::make_shared<Context>(state.identity,process.at("root").at("id"),definitions,clock);
  std::ifstream input(xml_path); std::string text((std::istreambuf_iterator<char>(input)),{});
  Executor executor(text,context);context->publish(state);
  for (int i=0;i<5;++i) {require(executor.tick()!=BT::NodeStatus::SUCCESS,"unexecuted restored work became success");require(context->take_requests().empty(),"restored BT generated new work before explicit authorization");}
}
}  // namespace
int main(int argc, char** argv) {
  try {
    sequence_and_handover(); decisions_waits_and_interventions(); unknown_parallel_and_revocation(); xml_whitelist_and_queue_bound(); source_clock_and_frame_parser(); persistent_wait_and_obsolete_queue();
    if (argc == 3) compiled_example(argv[1], argv[2]);
    else if (argc == 4) restored_platform_frame(argv[1],argv[2],argv[3]);
    else if (argc != 1) throw std::runtime_error("expected resolved JSON, XML and optional restored frame paths");
    std::cout << "PASS: RX BT nodes, unknown/revocation/handover, strict XML, bounded requests" << (argc == 3 ? ", compiled five-step example" : (argc == 4 ? ", restored platform frame" : "")) << '\n';
    return 0;
  } catch (const std::exception& error) { std::cerr << "FAIL: " << error.what() << '\n'; return 1; }
}
