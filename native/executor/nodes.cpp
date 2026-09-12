#include "runtime.hpp"
#include <behaviortree_cpp/action_node.h>
#include <behaviortree_cpp/control_node.h>

namespace rx::bt {
namespace {
using Status = BT::NodeStatus;
class Leaf : public BT::ActionNodeBase {
 public:
  Leaf(const std::string& name, const BT::NodeConfig& config, std::shared_ptr<Context> context, Kind kind)
      : BT::ActionNodeBase(name, config), context_(std::move(context)), kind_(kind) {
    auto id = getInput<std::string>("node_id");
    if (!id) throw BT::RuntimeError("RX node_id required");
    id_ = id.value();
    if (context_->definition(id_).kind != kind_) throw BT::RuntimeError("RX node kind mismatch");
  }
  static BT::PortsList providedPorts() {
    return {BT::InputPort<std::string>("node_id"), BT::InputPort<std::string>("binding"),
            BT::InputPort<std::string>("condition_id"), BT::InputPort<std::string>("timeout_ns"),
            BT::InputPort<std::string>("procedure_digest")};
  }
  void halt() override { resetStatus(); } // BT halt is not a native cancel or handover proof.
 private:
  Status tick() override {
    const auto value = context_->observe(id_);
    if (!context_->usable() || value.unknown || !value.integrity_valid) return Status::RUNNING;
    if (kind_ == Kind::Operation) {
      if (!value.operation_id) { context_->enqueue(id_, RequestKind::SubmitOperation); return Status::RUNNING; }
      switch (value.outcome) {
        case Outcome::Succeeded:
          if (value.released) return Status::SUCCESS;
          context_->enqueue(id_, RequestKind::RequestHandover); return Status::RUNNING;
        case Outcome::Failed: case Outcome::Canceled: case Outcome::NotExecuted: return Status::FAILURE;
        case Outcome::Unresolved: case Outcome::None: return Status::RUNNING;
      }
    }
    if (kind_ == Kind::Wait) {
      if (value.wait == WaitState::Satisfied) return Status::SUCCESS;
      if (value.wait == WaitState::TimedOut) return Status::FAILURE;
      context_->enqueue(id_, RequestKind::BeginWait); return Status::RUNNING;
    }
    if (kind_ == Kind::Intervention) {
      if (value.clearance) return Status::SUCCESS;
      context_->enqueue(id_, RequestKind::RequestIntervention); return Status::RUNNING;
    }
    throw BT::RuntimeError("unsupported RX leaf kind");
  }
  std::shared_ptr<Context> context_;
  Kind kind_;
  std::string id_;
};
class Control : public BT::ControlNode {
 public:
  Control(const std::string& name, const BT::NodeConfig& config, std::shared_ptr<Context> context, Kind kind)
      : BT::ControlNode(name, config), context_(std::move(context)), kind_(kind) {
    auto id = getInput<std::string>("node_id");
    if (!id) throw BT::RuntimeError("RX node_id required");
    id_ = id.value();
    if (context_->definition(id_).kind != kind_) throw BT::RuntimeError("RX control kind mismatch");
  }
  static BT::PortsList providedPorts() { return {BT::InputPort<std::string>("node_id"), BT::InputPort<std::string>("condition_id")}; }
 private:
  Status tick() override {
    if (!context_->usable()) return Status::RUNNING;
    if (kind_ == Kind::Branch) {
      const auto choice = context_->observe(id_);
      if (choice.unknown || !choice.integrity_valid) return Status::RUNNING;
      if (!choice.branch) { context_->enqueue(id_, RequestKind::ResolveBranch); return Status::RUNNING; }
      if (childrenCount() != 2) throw BT::RuntimeError("RXBranch requires two branches");
      return children_nodes_.at(*choice.branch ? 0 : 1)->executeTick();
    }
    if (childrenCount() == 0) throw BT::RuntimeError("empty RX control node");
    if (kind_ == Kind::Sequence) {
      for (auto* child : children_nodes_) {
        auto result = child->executeTick();
        if (result == Status::RUNNING || result == Status::FAILURE) return result;
        if (result != Status::SUCCESS) throw BT::RuntimeError("RX child produced an unsupported status");
      }
      return Status::SUCCESS;
    }
    bool running = false, failed = false;
    if (has_problem(id_)) context_->suppress_new_for_tick();
    for (auto* child : children_nodes_) {
      auto result = child->executeTick();
      running |= result == Status::RUNNING; failed |= result == Status::FAILURE;
      if (result != Status::RUNNING && result != Status::FAILURE && result != Status::SUCCESS)
        throw BT::RuntimeError("RX child produced an unsupported status");
    }
    // No halt/reset of outstanding children just because a sibling failed.
    if (has_blocker(id_)) return Status::RUNNING;
    if (failed && !has_live_work(id_)) return Status::FAILURE;
    return running ? Status::RUNNING : failed ? Status::FAILURE : Status::SUCCESS;
  }
  bool has_problem(const std::string& id) const {
    const auto value = context_->observe(id);
    if (value.unknown || !value.integrity_valid || value.outcome == Outcome::Unresolved ||
        value.outcome == Outcome::Failed || value.outcome == Outcome::Canceled || value.outcome == Outcome::NotExecuted) return true;
    const auto& def = context_->definition(id);
    if (def.kind == Kind::Wait && value.wait == WaitState::TimedOut) return true;
    if (def.kind == Kind::Intervention && !value.clearance) return true;
    for (const auto& child : active_children(id, value)) if (has_problem(child)) return true;
    return false;
  }
  std::vector<std::string> active_children(const std::string& id, const Observation& value) const {
    const auto& def = context_->definition(id);
    if (def.kind != Kind::Branch) return def.children;
    if (!value.branch) return {};
    return {def.children.at(*value.branch ? 0 : 1)};
  }
  bool has_blocker(const std::string& id) const {
    const auto value = context_->observe(id);
    if (value.unknown || !value.integrity_valid || value.outcome == Outcome::Unresolved ||
        (context_->definition(id).kind == Kind::Intervention && !value.clearance)) return true;
    for (const auto& child : active_children(id, value)) if (has_blocker(child)) return true;
    return false;
  }
  bool has_live_work(const std::string& id) const {
    const auto value = context_->observe(id);
    if (value.operation_id && value.outcome == Outcome::None) return true;
    for (const auto& child : active_children(id, value)) if (has_live_work(child)) return true;
    return false;
  }
  std::shared_ptr<Context> context_;
  Kind kind_;
  std::string id_;
};
}  // namespace
void register_nodes(BT::BehaviorTreeFactory& factory, std::shared_ptr<Context> context) {
  factory.registerNodeType<Control>("RXSequence", context, Kind::Sequence);
  factory.registerNodeType<Control>("RXParallelAll", context, Kind::ParallelAll);
  factory.registerNodeType<Control>("RXBranch", context, Kind::Branch);
  factory.registerNodeType<Leaf>("RXOperation", context, Kind::Operation);
  factory.registerNodeType<Leaf>("RXWait", context, Kind::Wait);
  factory.registerNodeType<Leaf>("RXIntervention", context, Kind::Intervention);
}
}  // namespace rx::bt
