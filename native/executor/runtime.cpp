#include "runtime.hpp"
#include <algorithm>
#include <stdexcept>
#include <tuple>

namespace rx::bt {
bool Identity::operator==(const Identity& other) const {
  return std::tie(run, executor_session, resolved_digest, visit, epoch) ==
         std::tie(other.run, other.executor_session, other.resolved_digest, other.visit, other.epoch);
}
Context::Context(Identity identity, std::string root, std::map<std::string, Definition> definitions, std::shared_ptr<FrameClock> clock)
    : clock_(std::move(clock)), identity_(std::move(identity)), root_(std::move(root)), definitions_(std::move(definitions)) {
  if (identity_.run.empty() || identity_.executor_session.empty() || identity_.resolved_digest.size() != 64 ||
      identity_.visit == 0 || identity_.epoch == 0 || definitions_.empty() || definitions_.size() > 4096 ||
      !definitions_.count(root_)) throw std::invalid_argument("invalid executor context");
}
const Definition& Context::definition(const std::string& node) const {
  auto it = definitions_.find(node);
  if (it == definitions_.end()) throw std::invalid_argument("node is not in the resolved process");
  return it->second;
}
void Context::publish(Frame frame) {
  std::lock_guard<std::mutex> lock(mutex_);
  try {
  if (!source_current(frame)) throw std::invalid_argument("expired or foreign source clock");
  if (!(frame.identity == identity_) || (incoming_ && frame.revision < incoming_->revision))
    throw std::invalid_argument("stale or foreign P frame");
  for (const auto& node : frame.eligible) definition(node);
  for (const auto& [node, value] : frame.nodes) {
    const auto& def = definition(node);
    if (value.operation_id && value.operation_id->empty()) throw std::invalid_argument("empty operation identity");
    if (value.released && (!value.operation_id || value.outcome == Outcome::None || !value.integrity_valid))
      throw std::invalid_argument("release contradicts operation state");
    if (value.outcome != Outcome::None && !value.operation_id) throw std::invalid_argument("outcome without operation identity");
    auto previous_outcome = outcomes_.find(node);
    if (previous_outcome != outcomes_.end() && previous_outcome->second != value.outcome)
      throw std::invalid_argument("immutable operation outcome changed");
    if (def.kind != Kind::Operation && (value.operation_id || value.outcome != Outcome::None || value.released))
      throw std::invalid_argument("operation fields on another node kind");
    if (def.kind != Kind::Branch && value.branch) throw std::invalid_argument("branch fields on another node kind");
    if (def.kind != Kind::Wait && value.wait != WaitState::Pending) throw std::invalid_argument("wait fields on another node kind");
    if (def.kind != Kind::Intervention && value.clearance) throw std::invalid_argument("clearance on another node kind");
    if (value.wait != WaitState::Pending && !value.decision_id) throw std::invalid_argument("wait result lacks P decision identity");
    auto previous_wait = waits_.find(node);
    if (previous_wait != waits_.end() && (!value.decision_id || previous_wait->second != std::make_pair(*value.decision_id, value.wait)))
      throw std::invalid_argument("committed wait result changed");
    if (value.decision_id && value.decision_id->empty()) throw std::invalid_argument("empty decision identity");
    if (value.clearance && value.clearance->empty()) throw std::invalid_argument("empty clearance identity");
    if (value.operation_id) {
      auto previous = operations_.find(node);
      if (previous != operations_.end() && previous->second != *value.operation_id)
        throw std::invalid_argument("operation identity changed at an existing activation");
      for (const auto& [other, operation] : operations_)
        if (other != node && operation == *value.operation_id) throw std::invalid_argument("operation shared across activations");
    }
    if (value.branch) {
      if (!value.decision_id) throw std::invalid_argument("branch has no committed decision identity");
      auto previous = decisions_.find(node);
      if (previous != decisions_.end() && previous->second != std::make_pair(*value.decision_id, *value.branch))
        throw std::invalid_argument("committed branch decision changed");
    }
  }
  if (frame.complete) {
    for (const auto& [node, operation] : operations_) {
      auto it = frame.nodes.find(node);
      if (it == frame.nodes.end() || it->second.operation_id != operation)
        throw std::invalid_argument("complete view lost an operation binding");
    }
    for (const auto& [node, choice] : decisions_) {
      auto it = frame.nodes.find(node);
      if (it == frame.nodes.end() || it->second.decision_id != choice.first || it->second.branch != choice.second)
        throw std::invalid_argument("complete view lost a committed branch choice");
    }
    for (const auto& [node, result] : waits_) {
      auto it = frame.nodes.find(node);
      if (it == frame.nodes.end() || it->second.decision_id != result.first || it->second.wait != result.second)
        throw std::invalid_argument("complete view lost a committed wait result");
    }
  }
  // Only mutate continuity memory after the entire incoming frame passed validation.
  for (const auto& [node, value] : frame.nodes) {
    if (value.operation_id) operations_[node] = *value.operation_id;
    if (value.outcome != Outcome::None) outcomes_[node] = value.outcome;
    if (value.branch) decisions_[node] = {*value.decision_id, *value.branch};
    if (value.wait != WaitState::Pending) waits_[node] = {*value.decision_id, value.wait};
  }
  // Undelivered suggestions already resolved by P no longer occupy the bounded queue.
  if (frame.complete) {
    queue_.erase(std::remove_if(queue_.begin(), queue_.end(), [&](const Request& request) {
      const auto it = frame.nodes.find(request.node); if (it == frame.nodes.end()) return false;
      const auto& value = it->second;
      switch (request.kind) {
        case RequestKind::SubmitOperation: return value.operation_id.has_value();
        case RequestKind::ResolveBranch: return value.branch.has_value();
        case RequestKind::BeginWait: return value.wait != WaitState::Pending;
        case RequestKind::RequestHandover: return value.released;
        case RequestKind::RequestIntervention: return value.clearance.has_value();
        case RequestKind::PauseExecutor: return false;
      }
      return false;
    }), queue_.end());
  }
  incoming_ = std::make_shared<const Frame>(std::move(frame));
  } catch (...) {
    halted_ = true;
    if (incoming_) {
      auto invalid = *incoming_; invalid.complete = false; invalid.current = false;
      incoming_ = std::make_shared<const Frame>(std::move(invalid));
    }
    const auto key = std::make_pair(root_, RequestKind::PauseExecutor);
    if (!requested_.count(key)) {
      queue_.push_front({identity_, root_, RequestKind::PauseExecutor, {}, 0}); requested_.insert(key);
    }
    throw;
  }
}
void Context::begin_tick() {
  std::lock_guard<std::mutex> lock(mutex_);
  tick_ = incoming_;
  suppress_new_ = false;
}
bool Context::source_current(const Frame& frame) const {
  if (!frame.source_deadline) return true; // legacy in-process synthetic tests have no source clock
  if (!clock_) return false;
  try {
    const auto now = clock_->now(); const auto& bound = *frame.source_deadline;
    return now.clock_id == bound.clock_id && now.ticks_ns >= bound.checked_at_ns && now.ticks_ns < bound.valid_until_ns;
  } catch (...) { return false; }
}
bool Context::usable() const { return tick_ && tick_->complete && tick_->current && std::chrono::steady_clock::now() < tick_->valid_until && source_current(*tick_); }
Observation Context::observe(const std::string& node) const {
  definition(node);
  if (!usable()) { Observation missing; missing.unknown = true; return missing; }
  auto it = tick_->nodes.find(node);
  return it == tick_->nodes.end() ? Observation{} : it->second;
}
bool Context::enqueue(const std::string& node, RequestKind kind) {
  const auto& def = definition(node);
  if (suppress_new_ && (kind == RequestKind::SubmitOperation || kind == RequestKind::ResolveBranch || kind == RequestKind::BeginWait)) return false;
  if (!usable() || !tick_->submission_authorized || !tick_->eligible.count(node)) return false;
  std::lock_guard<std::mutex> lock(mutex_);
  const auto key = std::make_pair(node, kind);
  if (halted_ || incoming_ != tick_ || !incoming_->current || !incoming_->submission_authorized) return false;
  if (requested_.count(key)) return true;
  if (queue_.size() >= 32) return false;
  queue_.push_back({identity_, node, kind, def.argument, def.timeout_ns});
  requested_.insert(key);
  return true;
}
void Context::pause(const std::string& node) {
  definition(node);
  std::lock_guard<std::mutex> lock(mutex_);
  halted_ = true;
  const auto key = std::make_pair(root_, RequestKind::PauseExecutor);
  if (requested_.count(key)) return;
  // One reserved control slot, independent of the 32 normal requests.
  queue_.push_front({identity_, root_, RequestKind::PauseExecutor, {}, 0});
  requested_.insert(key);
}
std::vector<Request> Context::take_requests(std::size_t limit) {
  if (limit == 0 || limit > 32) throw std::invalid_argument("request batch limit");
  std::lock_guard<std::mutex> lock(mutex_);
  std::vector<Request> result;
  auto remaining = queue_.size();
  while (!queue_.empty() && result.size() < limit && remaining-- > 0) {
    auto request = std::move(queue_.front()); queue_.pop_front();
    if (request.kind == RequestKind::PauseExecutor ||
        (!halted_ && incoming_ && incoming_->complete && incoming_->current && incoming_->submission_authorized &&
         std::chrono::steady_clock::now() < incoming_->valid_until && source_current(*incoming_) && incoming_->eligible.count(request.node)))
      result.push_back(std::move(request));
    else queue_.push_back(std::move(request));
  }
  return result;
}
Executor::Executor(std::string xml, std::shared_ptr<Context> context) : context_(std::move(context)) {
  if (!context_) throw std::invalid_argument("context required");
  validate_xml(xml, *context_);
  register_nodes(factory_, context_);
  tree_ = factory_.createTreeFromText(xml);
}
BT::NodeStatus Executor::tick() { context_->begin_tick(); return tree_.tickExactlyOnce(); }
void Executor::halt() { context_->pause(context_->root()); tree_.haltTree(); }
}  // namespace rx::bt
