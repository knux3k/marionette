#pragma once

#include "CoreMinimal.h"
#include "GameFramework/Character.h"
#include "DemoCharacter.generated.h"

UCLASS()
class DEMO_API ADemoCharacter : public ACharacter
{
	GENERATED_BODY()

public:
	ADemoCharacter();

	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Movement")
	float TurnRate = 45.f;

	UPROPERTY(VisibleAnywhere)
	TObjectPtr<class UBoxComponent> PickupBox;

	UPROPERTY(EditDefaultsOnly)
	FVector MuzzleOffset = FVector(100.f, 0.f, 50.f);

	int32 Ammo{30};

protected:
	virtual void BeginPlay() override;
	virtual void Tick(float DeltaTime) override;
	virtual void SetupPlayerInputComponent(class UInputComponent* PlayerInputComponent) override;

	void MoveForward(float Value);
	void Fire();

	UFUNCTION()
	void OnPickupOverlap(UPrimitiveComponent* OverlappedComp, AActor* OtherActor, UPrimitiveComponent* OtherComp, int32 OtherBodyIndex, bool bFromSweep, const FHitResult& SweepResult);

private:
	FTimerHandle FireTimer;
};
