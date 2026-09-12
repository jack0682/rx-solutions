#pragma once
#include <behaviortree_cpp/bt_factory.h>
#include <cstdint>
#include <chrono>
#include <deque>
#include <map>
#include <memory>
#include <mutex>
#include <optional>
#include <set>
#include <string>
#include <vector>

namespace rx::bt {
enum class Kind { Sequence, ParallelAll, Branch, Operation, Wait, Intervention };
enum class Outcome { None, Succeeded, Failed, Canceled, NotExecuted, Unresolved };
enum class WaitState { Pending, Satisfied, TimedOut };
struct Definition {
  Kind kind;
  std::vector<std::string> children;
  std::string argument;
  std::uint64_t timeout_ns = 0;
};
struct Identity {
  std::string run, executor_session, resolved_digest;
  std::uint64_t visit = 0;
  std::uint64_t epoch = 0;
  bool operator==(const Identity& other) const;
};
struct Observation {
  std::optional<std::string> operation_id;
  Outcome outcome = Outcome::None;
  bool unknown = false;
  bool integrity_valid = true;
  bool released = false;
  std::optional<bool> branch;
  std::optional<std::string> decision_id;
  WaitState wait = WaitState::Pending;
  std::optional<std::string> clearance;
};
struct ClockSample { std::string clock_id; std::uint64_t ticks_ns; };
class FrameClock { public: virtual ~FrameClock() = default; virtual ClockSample now() const = 0; };
struct SourceDeadline { std::string clock_id; std::uint64_t checked_at_ns; std::uint64_t valid_until_ns; };
struct Frame {
  Identity identity;
  std::uint64_t revision = 0;
  bool complete = false;
  bool current = false;
  bool submission_authorized = false;
  std::chrono::steady_clock::time_point valid_until{};
  std::optional<SourceDeadline> source_deadline;
  std::map<std::string, Observation> nodes;
  std::set<std::string> eligible;
};
enum class RequestKind { SubmitOperation, ResolveBranch, BeginWait, RequestIntervention, RequestHandover, PauseExecutor };
struct Request {
  Identity identity;
  std::string node;
  RequestKind kind;
  std::string argument;
  std::uint64_t timeout_ns = 0;
};

// A validated P client publishes complete views and drains requests outside the BT tick thread.
// This boundary deliberately contains no ROS/native calls, local timers or completion setters.
class Context {
 public:
  Context(Identity identity, std::string root, std::map<std::string, Definition> definitions, std::shared_ptr<FrameClock> clock = {});
  void publish(Frame frame);
  void begin_tick();
  Observation observe(const std::string& node) const;
  bool usable() const;
  void suppress_new_for_tick() { suppress_new_ = true; }
  bool enqueue(const std::string& node, RequestKind kind);
  void pause(const std::string& node);
  std::vector<Request> take_requests(std::size_t limit = 32);
  const Definition& definition(const std::string& node) const;
  const std::map<std::string, Definition>& definitions() const { return definitions_; }
  const std::string& root() const { return root_; }
 private:
  bool source_current(const Frame& frame) const;
  std::shared_ptr<FrameClock> clock_;
  Identity identity_;
  std::string root_;
  std::map<std::string, Definition> definitions_;
  mutable std::mutex mutex_;
  std::shared_ptr<const Frame> incoming_, tick_;
  std::map<std::string, std::string> operations_;
  std::map<std::string, Outcome> outcomes_;
  std::map<std::string, std::pair<std::string, bool>> decisions_;
  std::map<std::string, std::pair<std::string, WaitState>> waits_;
  std::deque<Request> queue_;
  std::set<std::pair<std::string, RequestKind>> requested_;
  bool halted_ = false;
  bool suppress_new_ = false;
};
// Strict local IPC parser; production packets always include a same-host clock deadline.
Frame decode_frame(const std::string& packet);
std::shared_ptr<FrameClock> linux_boottime_clock();
void register_nodes(BT::BehaviorTreeFactory& factory, std::shared_ptr<Context> context);
void validate_xml(const std::string& xml, const Context& context);
class Executor {
 public:
  Executor(std::string xml, std::shared_ptr<Context> context);
  BT::NodeStatus tick();
  void halt();
 private:
  std::shared_ptr<Context> context_;
  BT::BehaviorTreeFactory factory_;
  BT::Tree tree_;
};
}  // namespace rx::bt
