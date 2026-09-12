#include "runtime.hpp"
#include <behaviortree_cpp/contrib/json.hpp>
#include <charconv>
#include <fstream>
#include <iterator>
#include <limits>
#include <regex>
#include <stdexcept>
#ifdef __linux__
#include <time.h>
#endif
namespace rx::bt {
namespace {
using Json = nlohmann::json;
void require(bool ok, const char* reason) { if (!ok) throw std::invalid_argument(reason); }
void keys(const Json& value, std::initializer_list<const char*> expected) {
  require(value.is_object() && value.size() == expected.size(), "frame object field set differs");
  for (auto key : expected) require(value.contains(key), "frame object field missing");
}
std::string text(const Json& value) { require(value.is_string(), "frame string required"); return value.get<std::string>(); }
std::string uuid(const Json& value) {
  auto result = text(value);
  static const std::regex format("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}");
  require(std::regex_match(result, format), "canonical UUID required"); return result;
}
std::string name(const Json& value) {
  auto result = text(value);
  static const std::regex format("[A-Za-z0-9][A-Za-z0-9._/-]{0,127}");
  require(std::regex_match(result, format), "Name required"); return result;
}
std::uint64_t counter(const Json& value) {
  auto s = text(value); require(!s.empty() && (s.size() == 1 || s.front() != '0'), "canonical decimal string required");
  std::uint64_t result = 0; auto parsed = std::from_chars(s.data(), s.data() + s.size(), result);
  require(parsed.ec == std::errc{} && parsed.ptr == s.data() + s.size(), "uint64 string required"); return result;
}
bool boolean(const Json& value) { require(value.is_boolean(), "boolean required"); return value.get<bool>(); }
std::optional<std::string> optional_id(const Json& value) { if (value.is_null()) return {}; return uuid(value); }
Outcome outcome(const Json& value) {
  const auto s = text(value);
  if (s == "NONE") return Outcome::None;
  if (s == "SUCCEEDED") return Outcome::Succeeded;
  if (s == "FAILED") return Outcome::Failed;
  if (s == "CANCELED") return Outcome::Canceled;
  if (s == "NOT_EXECUTED") return Outcome::NotExecuted;
  if (s == "UNRESOLVED") return Outcome::Unresolved;
  throw std::invalid_argument("unknown outcome token");
}
WaitState wait(const Json& value) {
  const auto s = text(value);
  if (s == "PENDING") return WaitState::Pending;
  if (s == "SATISFIED") return WaitState::Satisfied;
  if (s == "TIMED_OUT") return WaitState::TimedOut;
  throw std::invalid_argument("unknown wait token");
}
#ifdef __linux__
class BoottimeClock final : public FrameClock {
 public:
  BoottimeClock() {
    std::ifstream input("/proc/sys/kernel/random/boot_id"); std::string boot; input >> boot;
    uuid(Json(boot)); id_ = "linux-boottime/" + boot;
  }
  ClockSample now() const override {
    timespec time{};
    if (clock_gettime(CLOCK_BOOTTIME, &time) != 0 || time.tv_sec < 0 || time.tv_nsec < 0 || time.tv_nsec >= 1000000000)
      throw std::runtime_error("CLOCK_BOOTTIME unavailable");
    auto seconds = static_cast<std::uint64_t>(time.tv_sec); auto nanos = static_cast<std::uint64_t>(time.tv_nsec);
    if (seconds > (std::numeric_limits<std::uint64_t>::max() - nanos) / 1000000000ULL) throw std::runtime_error("boottime overflow");
    return {id_, seconds * 1000000000ULL + nanos};
  }
 private: std::string id_;
};
#endif
}
Frame decode_frame(const std::string& packet) {
  require(packet.size() <= 1000000, "frame packet too large");
  std::vector<std::set<std::string>> objects;
  auto parsed = Json::parse(packet, [&](int depth, Json::parse_event_t event, Json& value) {
    require(depth <= 64, "frame nesting limit");
    if (event == Json::parse_event_t::object_start) objects.emplace_back();
    if (event == Json::parse_event_t::key) require(!objects.empty() && objects.back().insert(value.get<std::string>()).second, "duplicate JSON key");
    if (event == Json::parse_event_t::object_end) objects.pop_back();
    return true;
  });
  keys(parsed, {"schema", "identity", "revision", "complete", "current", "submission_authorized", "remaining_validity_ns", "source", "nodes", "eligible"});
  require(text(parsed.at("schema")) == "rx.bt-frame.v1", "frame schema differs");
  const auto& identity = parsed.at("identity"); keys(identity, {"run", "executor_session", "resolved_digest", "visit", "epoch"});
  auto digest = text(identity.at("resolved_digest"));
  require(digest.size() == 64 && digest.find_first_not_of("0123456789abcdef") == std::string::npos, "digest required");
  Frame result; result.identity = {uuid(identity.at("run")), uuid(identity.at("executor_session")), digest, counter(identity.at("visit")), counter(identity.at("epoch"))};
  result.revision = counter(parsed.at("revision"));
  require(result.revision > 0 && result.identity.visit > 0 && result.identity.epoch > 0, "positive frame position required");
  result.complete = boolean(parsed.at("complete")); result.current = boolean(parsed.at("current")); result.submission_authorized = boolean(parsed.at("submission_authorized"));
  const auto remaining = counter(parsed.at("remaining_validity_ns")); require(remaining > 0 && remaining <= 100000000, "frame validity limit");
  const auto& source = parsed.at("source"); keys(source, {"clock_id", "checked_at_ns", "valid_until_ns"});
  SourceDeadline bound{text(source.at("clock_id")), counter(source.at("checked_at_ns")), counter(source.at("valid_until_ns"))};
  require(!bound.clock_id.empty() && bound.clock_id.size() <= 256 && bound.valid_until_ns > bound.checked_at_ns && bound.valid_until_ns - bound.checked_at_ns <= 100000000 && remaining <= bound.valid_until_ns - bound.checked_at_ns, "invalid source deadline");
  result.source_deadline = bound; result.valid_until = std::chrono::steady_clock::now() + std::chrono::nanoseconds(remaining);
  const auto& nodes = parsed.at("nodes"); require(nodes.is_object() && nodes.size() <= 4096, "frame node limit");
  for (auto entry = nodes.begin(); entry != nodes.end(); ++entry) {
    const auto& node = entry.value(); keys(node, {"operation_id", "outcome", "unknown", "integrity_valid", "released", "branch", "decision_id", "wait", "clearance"});
    Observation item; item.operation_id = optional_id(node.at("operation_id")); item.outcome = outcome(node.at("outcome"));
    item.unknown = boolean(node.at("unknown")); item.integrity_valid = boolean(node.at("integrity_valid")); item.released = boolean(node.at("released"));
    if (!node.at("branch").is_null()) item.branch = boolean(node.at("branch"));
    item.decision_id = optional_id(node.at("decision_id")); item.wait = wait(node.at("wait")); item.clearance = optional_id(node.at("clearance"));
    result.nodes.emplace(name(Json(entry.key())), std::move(item));
  }
  const auto& eligible = parsed.at("eligible"); require(eligible.is_array() && eligible.size() <= 4096, "eligible limit");
  for (const auto& entry : eligible) require(result.eligible.insert(name(entry)).second, "duplicate eligible node");
  return result;
}
std::shared_ptr<FrameClock> linux_boottime_clock() {
#ifdef __linux__
  return std::make_shared<BoottimeClock>();
#else
  throw std::runtime_error("Linux boottime clock requires a Linux host");
#endif
}
} // namespace rx::bt
